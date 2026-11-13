//! Asynchronous playbook execution engine with connection pooling, privilege escalation, and retries.

use super::helpers::{
    LoopItemOutcome, build_loop_register_value, build_single_register_value, calculate_backoff,
    eval_condition, get_loop_items, partition_hosts, resolve_play_hosts, task_matches_tags,
    task_matches_tags_direct,
};
use crate::connector::{
    Connector, ConnectorError, PrivilegeConfig, TargetConfig, create_connector,
};
use crate::playbook::{Play, Playbook};
use crate::tasks::{Task, TaskModuleError, TaskResult, execute_task_module, task_needs_hostvars};
use futures_util::future::BoxFuture;
use serde_json::Value;
use std::collections::HashMap;
use std::hash::BuildHasher;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, OnceCell, Semaphore};

/// Shared, mutex-guarded remote or local [`Connector`] instance for a single target host.
pub struct PooledConnector {
    host: String,
    config: TargetConfig,
    inner: Mutex<Connector>,
}

impl PooledConnector {
    async fn new(host: String, config: TargetConfig) -> Result<Self, String> {
        let connector = create_connector(&config)
            .await
            .map_err(|e| format!("failed to instantiate connector for {host}: {e}"))?;
        Ok(Self {
            host,
            config,
            inner: Mutex::new(connector),
        })
    }

    async fn reconnect(&self) -> Result<(), String> {
        let new_conn = create_connector(&self.config)
            .await
            .map_err(|e| format!("reconnect failed for {}: {e}", self.host))?;
        let mut guard = self.inner.lock().await;
        *guard = new_conn;
        Ok(())
    }

    async fn close(&self) -> Result<(), String> {
        let guard = self.inner.lock().await;
        guard.close().await.map_err(|e| e.to_string())
    }
}

type ConnectingMap = HashMap<String, Arc<OnceCell<Arc<PooledConnector>>>>;

#[derive(Debug, Clone)]
struct PlayExecContext {
    play_become: Option<bool>,
    play_become_user: Option<String>,
    batch_idx: usize,
    primary_host: String,
    tag_filter: Vec<String>,
}

enum AttemptOutcome {
    Completed(TaskResult),
    TimedOut(u64),
    Transport(ConnectorError),
}

fn compute_effective_privilege(
    base_priv: Option<&PrivilegeConfig>,
    play_become: Option<bool>,
    play_become_user: Option<&str>,
    task: &Task,
    host_vars: &Value,
) -> Option<PrivilegeConfig> {
    let become_enabled = task
        .r#become
        .or(play_become)
        .or_else(|| base_priv.map(|p| p.become_enabled))
        .unwrap_or(false);
    if !become_enabled {
        return None;
    }
    let become_user = task
        .become_user
        .clone()
        .or_else(|| play_become_user.map(str::to_string))
        .or_else(|| {
            host_vars
                .get("ansible_become_user")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| base_priv.and_then(|p| p.become_user.clone()));
    let become_password = host_vars
        .get("ansible_become_pass")
        .or_else(|| host_vars.get("ansible_become_password"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| base_priv.and_then(|p| p.become_password.clone()));
    Some(PrivilegeConfig {
        become_enabled: true,
        become_user,
        become_password,
    })
}

fn describe_loop_item(item: &Value) -> String {
    match item {
        Value::String(s) => s.clone(),
        Value::Object(obj) => obj
            .get("pub")
            .and_then(Value::as_str)
            .or_else(|| obj.get("name").and_then(Value::as_str))
            .unwrap_or("item")
            .to_string(),
        _ => serde_json::to_string(item).unwrap_or_default(),
    }
}

/// Coordinates multi-host playbook execution, connection pooling, variable buses, and concurrency.
pub struct ExecutionEngine {
    /// Resolved inventory groups and initial host variables.
    pub inventory: crate::inventory::FlatInventory,
    /// Per-host connection and privilege configurations.
    pub target_configs: HashMap<String, TargetConfig>,
    /// Shared per-host runtime variable store updated by `register` and play `vars`.
    pub host_vars_bus: Arc<Mutex<HashMap<String, Value>>>,
    /// Global registered results exposed under `.global.<name>`.
    pub global_results: Arc<Mutex<HashMap<String, Value>>>,
    /// Concurrency semaphore bounding parallel host executions (`--forks`).
    pub semaphore: Arc<Semaphore>,
    /// Active pooled connections keyed by target hostname.
    pub connection_pool: Arc<Mutex<HashMap<String, Arc<PooledConnector>>>>,
    connecting: Arc<Mutex<ConnectingMap>>,
    /// Optional parent directory of the inventory file exposed as `.inventory_dir`.
    pub inventory_dir: Option<PathBuf>,
    /// Optional tag allowlist (`--tags`); empty means run all tasks.
    pub tag_filter: Vec<String>,
    /// Optional default per-task timeout in seconds.
    pub task_timeout_secs: Option<u64>,
}

impl ExecutionEngine {
    /// Constructs a new [`ExecutionEngine`] from a resolved inventory and target connection map.
    #[must_use]
    pub fn new<S: BuildHasher>(
        inventory: crate::inventory::FlatInventory,
        target_configs: HashMap<String, TargetConfig, S>,
        forks: usize,
        inventory_dir: Option<PathBuf>,
    ) -> Self {
        let mut configs: HashMap<String, TargetConfig> = target_configs.into_iter().collect();
        configs
            .entry("localhost".to_string())
            .or_insert_with(|| TargetConfig {
                host: "127.0.0.1".to_string(),
                connector_type: Some(crate::connector::ConnectorType::Local),
                ssh: None,
                privilege: None,
            });
        let mut bus_map = HashMap::with_capacity(inventory.host_vars.len());
        for (h, vars) in &inventory.host_vars {
            let mut h_vars = serde_json::Map::new();
            for (k, v) in vars {
                h_vars.insert(k.clone(), v.clone());
            }
            bus_map.insert(h.clone(), Value::Object(h_vars));
        }
        Self {
            inventory,
            target_configs: configs,
            host_vars_bus: Arc::new(Mutex::new(bus_map)),
            global_results: Arc::new(Mutex::new(HashMap::new())),
            semaphore: Arc::new(Semaphore::new(forks.max(1))),
            connection_pool: Arc::new(Mutex::new(HashMap::new())),
            connecting: Arc::new(Mutex::new(HashMap::new())),
            inventory_dir,
            tag_filter: Vec::new(),
            task_timeout_secs: None,
        }
    }

    async fn remove_inflight_cell(&self, host: &str, cell: &Arc<OnceCell<Arc<PooledConnector>>>) {
        let mut inflight = self.connecting.lock().await;
        if inflight.get(host).is_some_and(|c| Arc::ptr_eq(c, cell)) {
            inflight.remove(host);
        }
    }

    async fn get_or_create_connector(
        &self,
        host: &str,
        target_config: &TargetConfig,
    ) -> Result<Arc<PooledConnector>, String> {
        {
            let pool = self.connection_pool.lock().await;
            if let Some(conn) = pool.get(host) {
                return Ok(Arc::clone(conn));
            }
        }
        let cell = {
            let mut inflight = self.connecting.lock().await;
            Arc::clone(
                inflight
                    .entry(host.to_string())
                    .or_insert_with(|| Arc::new(OnceCell::new())),
            )
        };
        let host_owned = host.to_string();
        let config_owned = target_config.clone();
        let init_res = cell
            .get_or_try_init(|| async move {
                PooledConnector::new(host_owned, config_owned)
                    .await
                    .map(Arc::new)
            })
            .await;
        match init_res {
            Ok(conn) => {
                let cloned = Arc::clone(conn);
                {
                    let mut pool = self.connection_pool.lock().await;
                    pool.entry(host.to_string())
                        .or_insert_with(|| Arc::clone(&cloned));
                }
                self.remove_inflight_cell(host, &cell).await;
                Ok(cloned)
            }
            Err(e) => {
                self.remove_inflight_cell(host, &cell).await;
                Err(e)
            }
        }
    }

    async fn evict_connector(&self, host: &str) {
        let removed = {
            let mut pool = self.connection_pool.lock().await;
            pool.remove(host)
        };
        {
            let mut inflight = self.connecting.lock().await;
            inflight.remove(host);
        }
        if let Some(conn) = removed {
            let _ = tokio::time::timeout(Duration::from_secs(2), conn.close()).await;
        }
    }

    /// Executes all plays in `playbook` sequentially and closes pooled connections upon completion.
    ///
    /// # Errors
    ///
    /// Returns an error if target host resolution fails, `serial` is invalid, or any non-ignored
    /// task fails.
    pub async fn run_playbook(self: &Arc<Self>, playbook: &Playbook) -> Result<(), String> {
        let result = self.run_playbook_inner(playbook).await;
        let drained: Vec<(String, Arc<PooledConnector>)> = {
            let mut pool = self.connection_pool.lock().await;
            pool.drain().collect()
        };
        for (host, conn) in drained {
            if let Err(e) = tokio::time::timeout(Duration::from_secs(5), conn.close()).await {
                eprintln!("Warning: timed out closing connection for {host}: {e}");
            }
        }
        crate::logging::shutdown_logging();
        result
    }

    async fn run_playbook_inner(self: &Arc<Self>, playbook: &Playbook) -> Result<(), String> {
        for play in &playbook.0 {
            let play_name = play.name.as_deref().unwrap_or("unnamed play");
            crate::log_println!("\nPLAY [{play_name}]");
            let hosts = resolve_play_hosts(&play.hosts, &self.inventory)
                .map_err(|e| format!("play '{play_name}': {e}"))?;
            crate::log_println!("Target hosts: {hosts:?}");
            let batches = partition_hosts(&hosts, play.serial.as_ref())
                .map_err(|e| format!("play '{play_name}': {e}"))?;
            if hosts.is_empty() {
                crate::log_println!(
                    "Skipping play '{play_name}': group '{}' has 0 hosts",
                    play.hosts
                );
                continue;
            }
            self.inject_play_vars(&hosts, play.vars.as_ref()).await;
            let primary_host = hosts.first().cloned().unwrap_or_default();
            for (batch_idx, batch) in batches.iter().enumerate() {
                self.run_play_batch(play, batch, batch_idx, &primary_host)
                    .await?;
            }
        }
        Ok(())
    }

    async fn inject_play_vars(&self, hosts: &[String], play_vars: Option<&HashMap<String, Value>>) {
        let Some(p_vars) = play_vars else {
            return;
        };
        let mut bus = self.host_vars_bus.lock().await;
        for host in hosts {
            let entry = bus.entry(host.clone()).or_insert_with(|| {
                serde_json::json!({
                    "inventory_hostname": host
                })
            });
            if let Some(obj) = entry.as_object_mut() {
                for (k, v) in p_vars {
                    obj.insert(k.clone(), v.clone());
                }
            }
        }
    }

    async fn run_play_batch(
        self: &Arc<Self>,
        play: &Play,
        batch: &[String],
        batch_idx: usize,
        primary_host: &str,
    ) -> Result<(), String> {
        let Some(ref tasks) = play.tasks else {
            return Ok(());
        };
        let base_ctx = PlayExecContext {
            play_become: play.r#become,
            play_become_user: play.become_user.clone(),
            batch_idx,
            primary_host: primary_host.to_string(),
            tag_filter: self.tag_filter.clone(),
        };
        for task in tasks {
            if !task_matches_tags(task, &base_ctx.tag_filter) {
                continue;
            }
            let run_once = task.run_once.unwrap_or(false);
            if run_once && batch_idx > 0 {
                continue;
            }
            let task_name = task.name.as_deref().unwrap_or("unnamed task");
            crate::log_println!("\nTASK [{task_name}]");
            let targets: Vec<String> = if run_once {
                let first = batch.first().ok_or("batch is empty")?;
                vec![first.clone()]
            } else {
                batch.to_vec()
            };
            let mut handles = Vec::with_capacity(targets.len());
            for host in targets {
                let engine = Arc::clone(self);
                let t = task.clone();
                let ctx = base_ctx.clone();
                handles.push(tokio::spawn(async move {
                    engine.run_task_on_host(&t, &host, &ctx).await
                }));
            }
            let mut has_failure = false;
            for h in handles {
                match h.await {
                    Ok(Ok(res)) if !res.failed => {}
                    _ => has_failure = true,
                }
            }
            if has_failure
                && !task.ignore_errors.unwrap_or(false)
                && !play.ignore_errors.unwrap_or(false)
            {
                return Err("Playbook aborted due to task failure".to_string());
            }
        }
        Ok(())
    }

    async fn run_block_phase(
        &self,
        tasks: &[Task],
        host: &str,
        ctx: &PlayExecContext,
        any_changed: &mut bool,
        stop_on_error: bool,
    ) -> Result<(), String> {
        let mut phase_err: Option<String> = None;
        for sub_task in tasks {
            if !task_matches_tags(sub_task, &ctx.tag_filter) {
                continue;
            }
            if sub_task.run_once.unwrap_or(false) && (ctx.batch_idx > 0 || host != ctx.primary_host)
            {
                continue;
            }
            match self.run_task_on_host(sub_task, host, ctx).await {
                Ok(res) => {
                    if res.changed {
                        *any_changed = true;
                    }
                    if res.failed && !sub_task.ignore_errors.unwrap_or(false) {
                        phase_err = Some(res.stderr);
                        if stop_on_error {
                            break;
                        }
                    }
                }
                Err(e) => {
                    if !sub_task.ignore_errors.unwrap_or(false) {
                        phase_err = Some(e);
                        if stop_on_error {
                            break;
                        }
                    }
                }
            }
        }
        match phase_err {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    fn execute_block<'a>(
        &'a self,
        task: &'a Task,
        block_tasks: &'a [Task],
        host: &'a str,
        ctx: &'a PlayExecContext,
    ) -> BoxFuture<'a, Result<TaskResult, String>> {
        Box::pin(async move {
            let child_filter = if task_matches_tags_direct(task, &ctx.tag_filter) {
                Vec::new()
            } else {
                ctx.tag_filter.clone()
            };
            let sub_ctx = PlayExecContext {
                play_become: task.r#become.or(ctx.play_become),
                play_become_user: task
                    .become_user
                    .clone()
                    .or_else(|| ctx.play_become_user.clone()),
                batch_idx: ctx.batch_idx,
                primary_host: ctx.primary_host.clone(),
                tag_filter: child_filter,
            };
            let mut any_changed = false;
            let mut last_err = String::new();
            let mut block_failed = false;
            if let Err(e) = self
                .run_block_phase(block_tasks, host, &sub_ctx, &mut any_changed, true)
                .await
            {
                block_failed = true;
                last_err = e;
            }
            if block_failed && let Some(ref rescue) = task.rescue {
                match self
                    .run_block_phase(rescue, host, &sub_ctx, &mut any_changed, true)
                    .await
                {
                    Ok(()) => {
                        block_failed = false;
                        last_err.clear();
                    }
                    Err(e) => {
                        last_err = e;
                    }
                }
            }
            let mut always_failed = false;
            if let Some(ref always) = task.always
                && let Err(e) = self
                    .run_block_phase(always, host, &sub_ctx, &mut any_changed, false)
                    .await
            {
                always_failed = true;
                last_err = e;
            }
            let failed = block_failed || always_failed;
            Ok(TaskResult {
                stdout: String::new(),
                stderr: if failed { last_err } else { String::new() },
                exit_code: i32::from(failed),
                changed: any_changed,
                failed,
            })
        })
    }

    fn run_task_on_host<'a>(
        &'a self,
        task: &'a Task,
        host: &'a str,
        ctx: &'a PlayExecContext,
    ) -> BoxFuture<'a, Result<TaskResult, String>> {
        Box::pin(async move {
            let task_name = task.name.as_deref().unwrap_or("unnamed task");
            crate::logging::start_task(host, task_name);
            let result = self.run_task_on_host_inner(task, host, ctx).await;
            match &result {
                Ok(r) if r.failed => {
                    crate::logging::finish_task_failed(host, task_name, &r.stderr);
                    if host == "localhost" {
                        crate::log_println!(
                            "fatal: FAILED! => stdout: {}, stderr: {}",
                            r.stdout,
                            r.stderr
                        );
                    } else {
                        crate::log_println!(
                            "fatal: [{host}]: FAILED! => stdout: {}, stderr: {}",
                            r.stdout,
                            r.stderr
                        );
                    }
                }
                Ok(r) => {
                    crate::logging::finish_task_success(host, task_name);
                    let label = match (r.changed, host == "localhost") {
                        (true, true) => "changed".to_string(),
                        (true, false) => format!("changed: [{host}]"),
                        (false, true) => "ok".to_string(),
                        (false, false) => format!("ok: [{host}]"),
                    };
                    crate::log_println!("{label}");
                }
                Err(e) => {
                    crate::logging::finish_task_failed(host, task_name, e);
                    if host == "localhost" {
                        crate::log_println!("fatal: FAILED! => {e}");
                    } else {
                        crate::log_println!("fatal: [{host}]: FAILED! => {e}");
                    }
                }
            }
            result
        })
    }

    async fn resolve_delegate_config(&self, delegate_host: &str) -> Result<TargetConfig, String> {
        if let Some(tc) = self.target_configs.get(delegate_host) {
            return Ok(tc.clone());
        }
        if delegate_host == "localhost" || crate::connector::is_local_ip(delegate_host).await {
            return Ok(TargetConfig {
                host: if delegate_host == "localhost" {
                    "127.0.0.1".to_string()
                } else {
                    delegate_host.to_string()
                },
                connector_type: Some(crate::connector::ConnectorType::Local),
                ssh: None,
                privilege: None,
            });
        }
        Err(format!(
            "delegate_to host '{delegate_host}' not in inventory and not local"
        ))
    }

    async fn resolve_delegate_target(
        &self,
        task: &Task,
        host: &str,
        host_vars: &Value,
    ) -> Result<(String, TargetConfig), String> {
        let delegate_host = if let Some(ref raw_delegate) = task.delegate_to {
            let rendered = crate::template::render_template_string(
                raw_delegate,
                &crate::template::json_to_gtmpl(host_vars),
            )
            .map_err(|e| format!("failed to render delegate_to '{raw_delegate}': {e}"))?;
            let trimmed = rendered.trim();
            if trimmed.is_empty() {
                return Err(format!(
                    "delegate_to '{raw_delegate}' rendered to an empty host name"
                ));
            }
            trimmed.to_string()
        } else {
            host.to_string()
        };
        let target_config = self.resolve_delegate_config(&delegate_host).await?;
        Ok((delegate_host, target_config))
    }

    fn run_task_on_host_inner<'a>(
        &'a self,
        task: &'a Task,
        host: &'a str,
        ctx: &'a PlayExecContext,
    ) -> BoxFuture<'a, Result<TaskResult, String>> {
        Box::pin(async move {
            if task.run_once.unwrap_or(false) && (ctx.batch_idx > 0 || host != ctx.primary_host) {
                return Ok(TaskResult {
                    stdout: "skipped".to_string(),
                    stderr: String::new(),
                    exit_code: 0,
                    changed: false,
                    failed: false,
                });
            }
            let mut host_vars = self.build_host_vars(task, host).await;
            let has_loop = task.r#loop.is_some() || task.with_items.is_some();
            if !has_loop && let Some(ref when_cond) = task.when {
                match eval_condition(when_cond, &host_vars) {
                    Ok(false) => {
                        return Ok(TaskResult {
                            stdout: "skipped".to_string(),
                            stderr: String::new(),
                            exit_code: 0,
                            changed: false,
                            failed: false,
                        });
                    }
                    Ok(true) => {}
                    Err(e) => {
                        return Ok(TaskResult {
                            stdout: String::new(),
                            stderr: e,
                            exit_code: 1,
                            changed: false,
                            failed: true,
                        });
                    }
                }
            }
            if let Some(ref block_tasks) = task.block {
                return self.execute_block(task, block_tasks, host, ctx).await;
            }
            let _permit = self.semaphore.acquire().await.map_err(|e| e.to_string())?;
            if let Some(items) = get_loop_items(task, &host_vars)? {
                let loop_res = self
                    .execute_loop_task(task, host, &items, &mut host_vars, ctx)
                    .await;
                if let Some(obj) = host_vars.as_object_mut() {
                    obj.remove("item");
                }
                self.merge_host_vars_back(host, &host_vars).await;
                let (outcomes, final_res) = loop_res?;
                if task.register.is_some() {
                    let reg_val =
                        build_loop_register_value(&outcomes, task.register_type.as_deref());
                    self.store_register_value(task, host, reg_val).await;
                }
                Ok(final_res)
            } else {
                let (delegate_host, target_config) =
                    self.resolve_delegate_target(task, host, &host_vars).await?;
                let final_res = self
                    .execute_with_retries(task, &delegate_host, &target_config, &mut host_vars, ctx)
                    .await?;
                self.merge_host_vars_back(host, &host_vars).await;
                if task.register.is_some() {
                    let reg_val =
                        build_single_register_value(&final_res, task.register_type.as_deref());
                    self.store_register_value(task, host, reg_val).await;
                }
                Ok(final_res)
            }
        })
    }

    async fn execute_loop_task(
        &self,
        task: &Task,
        host: &str,
        items: &[Value],
        host_vars: &mut Value,
        ctx: &PlayExecContext,
    ) -> Result<(Vec<LoopItemOutcome>, TaskResult), String> {
        let mut outcomes = Vec::with_capacity(items.len());
        let mut any_failed = false;
        let mut any_changed = false;
        let mut stdout_accum = Vec::new();
        let mut stderr_accum = Vec::new();
        let total = u32::try_from(items.len()).unwrap_or(1).max(1);
        let task_name = task.name.as_deref().unwrap_or("unnamed task");
        let break_on_error = task.break_on_error.unwrap_or(false);
        for (idx, item) in items.iter().enumerate() {
            if let Some(obj) = host_vars.as_object_mut() {
                obj.insert("item".to_string(), item.clone());
            }
            if let Some(ref when_cond) = task.when {
                match eval_condition(when_cond, host_vars) {
                    Ok(false) => {
                        outcomes.push(LoopItemOutcome {
                            item: item.clone(),
                            result: TaskResult {
                                stdout: "skipped".to_string(),
                                stderr: String::new(),
                                exit_code: 0,
                                changed: false,
                                failed: false,
                            },
                            skipped: true,
                        });
                        continue;
                    }
                    Ok(true) => {}
                    Err(e) => {
                        any_failed = true;
                        stderr_accum
                            .push(format!("Item: {}, Error: {e}", describe_loop_item(item)));
                        outcomes.push(LoopItemOutcome {
                            item: item.clone(),
                            result: TaskResult {
                                stdout: String::new(),
                                stderr: e,
                                exit_code: 1,
                                changed: false,
                                failed: true,
                            },
                            skipped: false,
                        });
                        if break_on_error {
                            break;
                        }
                        continue;
                    }
                }
            }
            let pct = u32::try_from(idx).unwrap_or(0).saturating_mul(100) / total;
            crate::logging::update_task(host, task_name, pct, &describe_loop_item(item));
            let (delegate_host, target_config) =
                self.resolve_delegate_target(task, host, host_vars).await?;
            let iter_res = self
                .execute_with_retries(task, &delegate_host, &target_config, host_vars, ctx)
                .await?;
            if iter_res.failed {
                any_failed = true;
                stderr_accum.push(format!(
                    "Item: {}, Error: {}",
                    serde_json::to_string(item).unwrap_or_default(),
                    iter_res.stderr.trim()
                ));
            }
            if iter_res.changed {
                any_changed = true;
            }
            if !iter_res.stdout.trim().is_empty() {
                stdout_accum.push(format!(
                    "Item: {}, Output: {}",
                    serde_json::to_string(item).unwrap_or_default(),
                    iter_res.stdout.trim()
                ));
            }
            let item_failed = iter_res.failed;
            outcomes.push(LoopItemOutcome {
                item: item.clone(),
                result: iter_res,
                skipped: false,
            });
            if item_failed && break_on_error {
                break;
            }
        }
        let final_res = TaskResult {
            stdout: stdout_accum.join("\n"),
            stderr: stderr_accum.join("\n"),
            exit_code: i32::from(any_failed),
            changed: any_changed,
            failed: any_failed,
        };
        Ok((outcomes, final_res))
    }

    async fn execute_single_attempt(
        &self,
        task: &Task,
        delegate_host: &str,
        target_config: &TargetConfig,
        host_vars: &mut Value,
        effective_priv: Option<PrivilegeConfig>,
    ) -> Result<AttemptOutcome, String> {
        let timeout_secs = task.timeout.or(self.task_timeout_secs);
        let pooled = self
            .get_or_create_connector(delegate_host, target_config)
            .await?;
        let mut conn_guard = pooled.inner.lock().await;
        let prev_priv = conn_guard.privilege().cloned();
        conn_guard.set_privilege(effective_priv);
        let module_res = match timeout_secs {
            Some(secs) if secs > 0 => tokio::time::timeout(
                Duration::from_secs(secs),
                execute_task_module(task, &conn_guard, host_vars),
            )
            .await
            .ok(),
            _ => Some(execute_task_module(task, &conn_guard, host_vars).await),
        };
        conn_guard.set_privilege(prev_priv);
        drop(conn_guard);
        match module_res {
            None => {
                self.evict_connector(delegate_host).await;
                Ok(AttemptOutcome::TimedOut(timeout_secs.unwrap_or(0)))
            }
            Some(Ok(res)) => Ok(AttemptOutcome::Completed(res)),
            Some(Err(TaskModuleError::Transport(te))) => Ok(AttemptOutcome::Transport(te)),
            Some(Err(TaskModuleError::Message(msg))) => Ok(AttemptOutcome::Completed(TaskResult {
                stdout: String::new(),
                stderr: msg,
                exit_code: 1,
                changed: false,
                failed: true,
            })),
        }
    }

    async fn execute_with_retries(
        &self,
        task: &Task,
        delegate_host: &str,
        target_config: &TargetConfig,
        host_vars: &mut Value,
        ctx: &PlayExecContext,
    ) -> Result<TaskResult, String> {
        let total_attempts = task.retries.unwrap_or(0).saturating_add(1);
        let delay = task.delay.unwrap_or(0);
        let effective_priv = compute_effective_privilege(
            target_config.privilege.as_ref(),
            ctx.play_become,
            ctx.play_become_user.as_deref(),
            task,
            host_vars,
        );
        let mut reconnected = false;
        let mut final_res = TaskResult::default();
        let mut attempt = 0;
        while attempt < total_attempts {
            if attempt > 0 {
                let attempt_u32 = u32::try_from(attempt).unwrap_or(1);
                let base_delay_ms = if delay == 0 {
                    1000
                } else {
                    delay.saturating_mul(1000)
                };
                let backoff_duration = calculate_backoff(attempt_u32, base_delay_ms, 60_000);
                tokio::time::sleep(backoff_duration).await;
            }
            let outcome = self
                .execute_single_attempt(
                    task,
                    delegate_host,
                    target_config,
                    host_vars,
                    effective_priv.clone(),
                )
                .await?;
            match outcome {
                AttemptOutcome::Completed(res) => {
                    final_res = res;
                }
                AttemptOutcome::TimedOut(secs) => {
                    final_res = TaskResult {
                        stdout: String::new(),
                        stderr: format!("task timed out after {secs} seconds"),
                        exit_code: 1,
                        changed: false,
                        failed: true,
                    };
                }
                AttemptOutcome::Transport(te) => {
                    if !reconnected && te.is_transport_error() {
                        tracing::warn!(
                            "transport error on {delegate_host}, reconnecting once: {te}"
                        );
                        if let Ok(pooled) = self
                            .get_or_create_connector(delegate_host, target_config)
                            .await
                            && pooled.reconnect().await.is_ok()
                        {
                            reconnected = true;
                            continue;
                        }
                        self.evict_connector(delegate_host).await;
                    }
                    if attempt + 1 < total_attempts {
                        self.evict_connector(delegate_host).await;
                        attempt += 1;
                        continue;
                    }
                    return Err(format!("task execution failed: {te}"));
                }
            }
            Self::apply_failed_when(task, host_vars, &mut final_res);
            if !final_res.failed {
                break;
            }
            attempt += 1;
        }
        Ok(final_res)
    }

    fn apply_failed_when(task: &Task, host_vars: &Value, final_res: &mut TaskResult) {
        let Some(ref fw) = task.failed_when else {
            return;
        };
        let mut eval_vars = host_vars.clone();
        if let Some(obj) = eval_vars.as_object_mut() {
            obj.insert(
                "stdout".to_string(),
                Value::String(final_res.stdout.clone()),
            );
            obj.insert(
                "stderr".to_string(),
                Value::String(final_res.stderr.clone()),
            );
            obj.insert("rc".to_string(), Value::Number(final_res.exit_code.into()));
        }
        match eval_condition(fw, &eval_vars) {
            Ok(failed) => final_res.failed = failed,
            Err(e) => {
                final_res.failed = true;
                final_res.stderr = format!("failed_when evaluation failed: {e}");
                final_res.exit_code = 1;
            }
        }
    }

    async fn build_host_vars(&self, task: &Task, host: &str) -> Value {
        let mut host_vars = {
            let bus = self.host_vars_bus.lock().await;
            bus.get(host).cloned().unwrap_or_else(|| {
                serde_json::json!({
                    "inventory_hostname": host
                })
            })
        };
        if let Some(ref defaults) = task.role_defaults
            && let Some(obj) = host_vars.as_object_mut()
        {
            for (k, v) in defaults {
                if !obj.contains_key(k) {
                    obj.insert(k.clone(), v.clone());
                }
            }
        }
        if let Some(ref params) = task.role_params
            && let Some(obj) = host_vars.as_object_mut()
        {
            for (k, v) in params {
                obj.insert(k.clone(), v.clone());
            }
        }
        {
            let globals = self.global_results.lock().await;
            let mut global_map = serde_json::Map::new();
            for (k, v) in globals.iter() {
                global_map.insert(k.clone(), v.clone());
            }
            if let Some(obj) = host_vars.as_object_mut() {
                obj.insert("global".to_string(), Value::Object(global_map));
            }
        }
        if task_needs_hostvars(task) {
            let bus = self.host_vars_bus.lock().await;
            let mut hostvars_map = serde_json::Map::new();
            for (hname, hval) in bus.iter() {
                hostvars_map.insert(hname.clone(), hval.clone());
            }
            if let Some(obj) = host_vars.as_object_mut() {
                obj.insert("hostvars".to_string(), Value::Object(hostvars_map));
            }
        } else if let Some(obj) = host_vars.as_object_mut() {
            obj.insert(
                "hostvars".to_string(),
                Value::Object(serde_json::Map::new()),
            );
        }
        if let Some(ref inv_dir) = self.inventory_dir
            && let Some(obj) = host_vars.as_object_mut()
        {
            obj.insert(
                "inventory_dir".to_string(),
                Value::String(inv_dir.to_string_lossy().to_string()),
            );
        }
        host_vars
    }

    async fn merge_host_vars_back(&self, host: &str, host_vars: &Value) {
        let mut bus = self.host_vars_bus.lock().await;
        if let Some(ref_val) = bus.get_mut(host)
            && let Some(obj) = ref_val.as_object_mut()
        {
            obj.remove("item");
            if let Some(new_obj) = host_vars.as_object() {
                for (k, v) in new_obj {
                    if k != "global" && k != "hostvars" && k != "item" {
                        obj.insert(k.clone(), v.clone());
                    }
                }
            }
        }
    }

    async fn store_register_value(&self, task: &Task, host: &str, reg_value: Value) {
        let Some(ref reg_name) = task.register else {
            return;
        };
        {
            let mut bus = self.host_vars_bus.lock().await;
            if let Some(ref_val) = bus.get_mut(host)
                && let Some(obj) = ref_val.as_object_mut()
            {
                obj.insert(reg_name.clone(), reg_value.clone());
            }
        }
        let set_global = task.set_global.unwrap_or(false) || task.run_once.unwrap_or(false);
        if set_global {
            let mut globals = self.global_results.lock().await;
            globals.insert(reg_name.clone(), reg_value);
        }
    }
}
