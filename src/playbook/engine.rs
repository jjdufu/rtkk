use super::helpers::{
    calculate_backoff, eval_condition_detailed, get_loop_items, partition_hosts, resolve_play_hosts,
    task_matches_tags,
};
use crate::connector::{Connector, ConnectorError, TargetConfig, create_connector};
use crate::playbook::Playbook;
use crate::tasks::{Task, TaskModuleError, TaskResult, execute_task_module, task_needs_hostvars};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, OnceCell, Semaphore};
pub(crate) struct PooledConnector {
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
        let mut guard = self.inner.lock().await;
        *guard = create_connector(&self.config)
            .await
            .map_err(|e| format!("reconnect failed for {}: {e}", self.host))?;
        Ok(())
    }
    async fn close(&self) -> Result<(), String> {
        let guard = self.inner.lock().await;
        guard.close().await.map_err(|e| e.to_string())
    }
}
type ConnectingMap =
    HashMap<String, Arc<OnceCell<Result<Arc<PooledConnector>, String>>>>;
pub(crate) struct ExecutionEngine {
    pub inventory: crate::inventory::FlatInventory,
    pub target_configs: HashMap<String, TargetConfig>,
    pub host_vars_bus: Arc<Mutex<HashMap<String, Value>>>,
    pub global_results: Arc<Mutex<HashMap<String, Value>>>,
    pub semaphore: Arc<Semaphore>,
    pub connection_pool: Arc<Mutex<HashMap<String, Arc<PooledConnector>>>>,
    connecting: Arc<Mutex<ConnectingMap>>,
    pub inventory_dir: Option<std::path::PathBuf>,
    pub tag_filter: Vec<String>,
    pub task_timeout_secs: Option<u64>,
}
impl ExecutionEngine {
    #[must_use]
    pub(crate) fn new(
        inventory: crate::inventory::FlatInventory,
        mut target_configs: HashMap<String, TargetConfig>,
        forks: usize,
        inventory_dir: Option<std::path::PathBuf>,
    ) -> Self {
        target_configs
            .entry("localhost".to_string())
            .or_insert_with(|| TargetConfig {
                host: "127.0.0.1".to_string(),
                connector_type: Some(crate::connector::ConnectorType::Local),
                ssh: None,
            });
        let mut bus_map = HashMap::new();
        for (h, vars) in &inventory.host_vars {
            let mut h_vars = serde_json::Map::new();
            for (k, v) in vars {
                h_vars.insert(k.clone(), v.clone());
            }
            bus_map.insert(h.clone(), Value::Object(h_vars));
        }
        Self {
            inventory,
            target_configs,
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
        let result = cell
            .get_or_init(|| async move {
                PooledConnector::new(host_owned, config_owned)
                    .await
                    .map(Arc::new)
            })
            .await;
        match result {
            Ok(conn) => {
                let mut pool = self.connection_pool.lock().await;
                pool.entry(host.to_string())
                    .or_insert_with(|| Arc::clone(conn));
                Ok(Arc::clone(conn))
            }
            Err(e) => Err(e.clone()),
        }
    }
    pub(crate) async fn run_playbook(self: &Arc<Self>, playbook: &Playbook) -> Result<(), String> {
        let result = self.run_playbook_inner(playbook).await;
        let mut pool = self.connection_pool.lock().await;
        for (host, conn) in pool.drain() {
            if let Err(e) = conn.close().await {
                eprintln!("Warning: failed to close connection for {host}: {e}");
            }
        }
        crate::logging::shutdown_logging();
        result
    }
    fn empty_hosts_policy_fail() -> bool {
        match std::env::var("RTKK_EMPTY_HOSTS") {
            Ok(v) => {
                let t = v.trim().to_ascii_lowercase();
                t != "skip" && t != "warn" && t != "ok"
            }
            Err(_) => true,
        }
    }
    async fn run_playbook_inner(self: &Arc<Self>, playbook: &Playbook) -> Result<(), String> {
        for play in &playbook.0 {
            let play_name = play.name.as_deref().unwrap_or("unnamed play");
            crate::log_println!("\nPLAY [{play_name}]");
            let hosts = resolve_play_hosts(&play.hosts, &self.inventory);
            crate::log_println!("Target hosts: {hosts:?}");
            if hosts.is_empty() {
                if Self::empty_hosts_policy_fail() {
                    return Err(format!(
                        "play '{play_name}': no hosts matched pattern '{}'",
                        play.hosts
                    ));
                }
                tracing::warn!(
                    "play '{play_name}': no hosts matched pattern '{}'; skipping (RTKK_EMPTY_HOSTS)",
                    play.hosts
                );
                continue;
            }
            if let Some(ref p_vars) = play.vars {
                let mut bus = self.host_vars_bus.lock().await;
                for host in &hosts {
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
            let batches = partition_hosts(&hosts, play.serial.as_ref());
            for batch in batches {
                if let Some(ref tasks) = play.tasks {
                    for task in tasks {
                        if !task_matches_tags(task, &self.tag_filter) {
                            continue;
                        }
                        let task_name = task.name.as_deref().unwrap_or("unnamed task");
                        crate::log_println!("\nTASK [{task_name}]");
                        let run_once = task.run_once.unwrap_or(false);
                        let targets = if run_once {
                            let first = batch.first().ok_or("batch is empty")?;
                            vec![first.clone()]
                        } else {
                            batch.clone()
                        };
                        let mut handles = Vec::new();
                        for host in targets {
                            let self_clone = Arc::clone(self);
                            let t = task.clone();
                            handles.push(tokio::spawn(async move {
                                self_clone.run_task_on_host(&t, &host).await
                            }));
                        }
                        let mut has_failure = false;
                        for h in handles {
                            match h.await {
                                Ok(Ok(res)) => {
                                    if res.failed {
                                        has_failure = true;
                                    }
                                }
                                _ => {
                                    has_failure = true;
                                }
                            }
                        }
                        if has_failure
                            && !task.ignore_errors.unwrap_or(false)
                            && !play.ignore_errors.unwrap_or(false)
                        {
                            return Err("Playbook aborted due to task failure".to_string());
                        }
                    }
                }
            }
        }
        Ok(())
    }
    async fn execute_block(
        &self,
        block_tasks: &[Task],
        rescue_tasks: Option<&Vec<Task>>,
        always_tasks: Option<&Vec<Task>>,
        host: &str,
    ) -> Result<TaskResult, String> {
        let mut block_failed = false;
        let mut block_res = TaskResult::default();
        for b_task in block_tasks {
            if !task_matches_tags(b_task, &self.tag_filter) {
                continue;
            }
            let res = self.run_task_on_host(b_task, host).await;
            if let Ok(r) = res {
                block_res = r;
                if block_res.failed {
                    block_failed = true;
                    break;
                }
            } else {
                block_failed = true;
                break;
            }
        }
        if block_failed {
            let mut rescued = true;
            if let Some(rescue) = rescue_tasks {
                for r_task in rescue {
                    if !task_matches_tags(r_task, &self.tag_filter) {
                        continue;
                    }
                    let res = self.run_task_on_host(r_task, host).await;
                    if res.is_err() || res.as_ref().is_ok_and(|r| r.failed) {
                        rescued = false;
                        break;
                    }
                }
            } else {
                rescued = false;
            }
            if rescued {
                block_res.failed = false;
            }
        }
        let mut always_failed = false;
        if let Some(always) = always_tasks {
            for a_task in always {
                if !task_matches_tags(a_task, &self.tag_filter) {
                    continue;
                }
                match self.run_task_on_host(a_task, host).await {
                    Ok(r) if r.failed => always_failed = true,
                    Err(_) => always_failed = true,
                    _ => {}
                }
            }
        }
        if always_failed {
            block_res.failed = true;
        }
        Ok(block_res)
    }
    async fn execute_loop_task(
        &self,
        task: &Task,
        connector: &Connector,
        host_vars: &mut Value,
        items: &[Value],
        host: &str,
    ) -> Result<(Vec<TaskResult>, TaskResult), ConnectorError> {
        let mut results_list = Vec::new();
        let mut final_res = TaskResult::default();
        let mut failed = false;
        let mut changed = false;
        let mut stdout_accum = Vec::new();
        let mut stderr_accum = Vec::new();
        let total = items.len();
        let task_name = task.name.as_deref().unwrap_or("unnamed task");
        let break_on_error = task.break_on_error.unwrap_or(false);
        for (idx, item) in items.iter().enumerate() {
            if let Some(obj) = host_vars.as_object_mut() {
                obj.insert("item".to_string(), item.clone());
            }
            let item_str = match item {
                Value::String(s) => s.clone(),
                Value::Object(obj) => obj
                    .get("pub")
                    .and_then(Value::as_str)
                    .or_else(|| obj.get("name").and_then(Value::as_str))
                    .unwrap_or_else(|| item.as_str().unwrap_or("item"))
                    .to_string(),
                _ => serde_json::to_string(item).unwrap_or_default(),
            };
            let pct = (u32::try_from(idx).unwrap_or(0) * 100) / u32::try_from(total).unwrap_or(1);
            crate::logging::update_task(host, task_name, pct, &item_str);
            let iteration_res = self.execute_with_retries(task, connector, host_vars).await?;
            if iteration_res.failed {
                failed = true;
                stderr_accum.push(format!(
                    "Item: {}, Error: {}",
                    serde_json::to_string(item).unwrap_or_default(),
                    iteration_res.stderr.trim()
                ));
            }
            if iteration_res.changed {
                changed = true;
            }
            if !iteration_res.stdout.trim().is_empty() {
                stdout_accum.push(format!(
                    "Item: {}, Output: {}",
                    serde_json::to_string(item).unwrap_or_default(),
                    iteration_res.stdout.trim()
                ));
            }
            let item_failed = iteration_res.failed;
            results_list.push(iteration_res);
            if item_failed && break_on_error {
                break;
            }
        }
        final_res.failed = failed;
        final_res.changed = changed;
        final_res.stdout = stdout_accum.join("\n");
        final_res.stderr = stderr_accum.join("\n");
        final_res.exit_code = i32::from(failed);
        Ok((results_list, final_res))
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
        if let Some(ref defaults) = task.role_defaults {
            if let Some(obj) = host_vars.as_object_mut() {
                for (k, v) in defaults {
                    if !obj.contains_key(k) {
                        obj.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        if let Some(ref params) = task.role_params {
            if let Some(obj) = host_vars.as_object_mut() {
                for (k, v) in params {
                    obj.insert(k.clone(), v.clone());
                }
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
        {
            let needs_hostvars = task_needs_hostvars(task);
            if needs_hostvars {
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
        }
        if let Some(ref inv_dir) = self.inventory_dir {
            if let Some(obj) = host_vars.as_object_mut() {
                obj.insert(
                    "inventory_dir".to_string(),
                    Value::String(inv_dir.to_string_lossy().to_string()),
                );
            }
        }
        host_vars
    }
    fn run_task_on_host<'a>(
        &'a self,
        task: &'a Task,
        host: &'a str,
    ) -> futures_util::future::BoxFuture<'a, Result<TaskResult, String>> {
        Box::pin(async move {
            let task_name = task.name.as_deref().unwrap_or("unnamed task");
            crate::logging::start_task(host, task_name);
            let result = self.run_task_on_host_inner(task, host).await;
            match &result {
                Ok(r) => {
                    if r.failed {
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
                    } else {
                        crate::logging::finish_task_success(host, task_name);
                        if r.changed {
                            if host == "localhost" {
                                crate::log_println!("changed");
                            } else {
                                crate::log_println!("changed: [{host}]");
                            }
                        } else if host == "localhost" {
                            crate::log_println!("ok");
                        } else {
                            crate::log_println!("ok: [{host}]");
                        }
                    }
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
        if crate::connector::is_local_ip(delegate_host).await || delegate_host == "localhost" {
            return Ok(TargetConfig {
                host: if delegate_host == "localhost" {
                    "127.0.0.1".to_string()
                } else {
                    delegate_host.to_string()
                },
                connector_type: Some(crate::connector::ConnectorType::Local),
                ssh: None,
            });
        }
        Err(format!(
            "delegate_to host '{delegate_host}' not in inventory and not local"
        ))
    }
    async fn run_on_connector(
        &self,
        task: &Task,
        host: &str,
        delegate_host: &str,
        pooled: &PooledConnector,
        host_vars: &mut Value,
    ) -> Result<(Vec<TaskResult>, TaskResult), String> {
        let mut reconnected = false;
        loop {
            let outcome = {
                let guard = pooled.inner.lock().await;
                let loop_items = get_loop_items(task, host_vars)?;
                if let Some(items) = loop_items {
                    match self
                        .execute_loop_task(task, &guard, host_vars, &items, host)
                        .await
                    {
                        Ok((r_list, f_res)) => (r_list, f_res, None::<ConnectorError>),
                        Err(te) => (Vec::new(), TaskResult::default(), Some(te)),
                    }
                } else {
                    match self.execute_with_retries(task, &guard, host_vars).await {
                        Ok(f_res) => (Vec::new(), f_res, None),
                        Err(te) => (Vec::new(), TaskResult::default(), Some(te)),
                    }
                }
            };
            if let Some(te) = outcome.2 {
                if !reconnected && te.is_transport_error() {
                    tracing::warn!("transport error on {delegate_host}, reconnecting once: {te}");
                    pooled.reconnect().await?;
                    reconnected = true;
                    continue;
                }
                return Err(format!("task execution failed: {te}"));
            }
            return Ok((outcome.0, outcome.1));
        }
    }
    async fn merge_host_vars_back(&self, host: &str, host_vars: &Value) {
        let mut bus = self.host_vars_bus.lock().await;
        if let Some(ref_val) = bus.get_mut(host) {
            if let Some(obj) = ref_val.as_object_mut() {
                if let Some(new_obj) = host_vars.as_object() {
                    for (k, v) in new_obj {
                        if k != "global" && k != "hostvars" {
                            obj.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
        }
    }
    fn run_task_on_host_inner<'a>(
        &'a self,
        task: &'a Task,
        host: &'a str,
    ) -> futures_util::future::BoxFuture<'a, Result<TaskResult, String>> {
        Box::pin(async move {
            let mut host_vars = self.build_host_vars(task, host).await;
            if let Some(ref when_cond) = task.when {
                match eval_condition_detailed(when_cond, &host_vars) {
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
                return self
                    .execute_block(
                        block_tasks,
                        task.rescue.as_ref(),
                        task.always.as_ref(),
                        host,
                    )
                    .await;
            }
            let delegate_host = task.delegate_to.as_deref().unwrap_or(host);
            let target_config = self.resolve_delegate_config(delegate_host).await?;
            let _permit = self.semaphore.acquire().await.map_err(|e| e.to_string())?;
            let pooled = self
                .get_or_create_connector(delegate_host, &target_config)
                .await?;
            let (results_list, final_res) = self
                .run_on_connector(task, host, delegate_host, &pooled, &mut host_vars)
                .await?;
            self.merge_host_vars_back(host, &host_vars).await;
            self.write_register_vars(task, host, &final_res, &results_list)
                .await;
            Ok(final_res)
        })
    }
    async fn write_register_vars(
        &self,
        task: &Task,
        host: &str,
        final_res: &TaskResult,
        results_list: &[TaskResult],
    ) {
        if let Some(ref reg_name) = task.register {
            let reg_value = if results_list.is_empty() {
                Self::register_object(final_res, task.register_type.as_ref())
            } else {
                let arr: Vec<Value> = results_list
                    .iter()
                    .map(|r| Self::register_object(r, task.register_type.as_ref()))
                    .collect();
                let mut map = serde_json::Map::new();
                map.insert("results".to_string(), Value::Array(arr.clone()));
                map.insert("stdout".to_string(), Value::String(final_res.stdout.clone()));
                map.insert("stderr".to_string(), Value::String(final_res.stderr.clone()));
                map.insert(
                    "rc".to_string(),
                    Value::Number(final_res.exit_code.into()),
                );
                map.insert("changed".to_string(), Value::Bool(final_res.changed));
                map.insert("failed".to_string(), Value::Bool(final_res.failed));
                map.insert(
                    "error".to_string(),
                    if final_res.failed {
                        Value::String(final_res.stderr.clone())
                    } else {
                        Value::Null
                    },
                );
                Value::Array(arr)
            };
            {
                let mut bus = self.host_vars_bus.lock().await;
                if let Some(ref_val) = bus.get_mut(host) {
                    if let Some(obj) = ref_val.as_object_mut() {
                        obj.insert(reg_name.clone(), reg_value.clone());
                    }
                }
            }
            let set_global = task.set_global.unwrap_or(false) || task.run_once.unwrap_or(false);
            if set_global {
                let mut globals = self.global_results.lock().await;
                globals.insert(reg_name.clone(), reg_value);
            }
        }
    }
    fn register_object(res: &TaskResult, register_type: Option<&String>) -> Value {
        let parsed_stdout = Self::parse_register_stdout(&res.stdout, register_type);
        serde_json::json!({
            "stdout": parsed_stdout,
            "stderr": res.stderr,
            "rc": res.exit_code,
            "changed": res.changed,
            "failed": res.failed,
            "error": if res.failed { Value::String(res.stderr.clone()) } else { Value::Null }
        })
    }
    async fn execute_with_retries(
        &self,
        task: &Task,
        connector: &Connector,
        host_vars: &mut Value,
    ) -> Result<TaskResult, ConnectorError> {
        let retries = task.retries.unwrap_or(0) + 1;
        let delay = task.delay.unwrap_or(0);
        let timeout_secs = task.timeout.or(self.task_timeout_secs);
        let mut final_res = TaskResult::default();
        for attempt in 0..retries {
            if attempt > 0 {
                let attempt_u32 = u32::try_from(attempt).unwrap_or(1);
                let base_delay_ms = if delay == 0 {
                    1000
                } else {
                    delay.saturating_mul(1000)
                };
                let max_delay_ms = 60000;
                let backoff_duration = calculate_backoff(attempt_u32, base_delay_ms, max_delay_ms);
                tokio::time::sleep(backoff_duration).await;
            }
            match execute_task_module_with_timeout(task, connector, host_vars, timeout_secs).await {
                Ok(res) => {
                    final_res = res;
                }
                Err(TaskModuleError::Transport(e)) => {
                    return Err(e);
                }
                Err(TaskModuleError::Message(e)) => {
                    final_res = TaskResult {
                        stdout: String::new(),
                        stderr: e,
                        exit_code: 1,
                        changed: false,
                        failed: true,
                    };
                }
            }
            if let Some(ref fw) = task.failed_when {
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
                match eval_condition_detailed(fw, &eval_vars) {
                    Ok(failed) => final_res.failed = failed,
                    Err(e) => {
                        final_res.failed = true;
                        final_res.stderr = format!("failed_when render failed: {e}");
                        final_res.exit_code = 1;
                    }
                }
            }
            if !final_res.failed {
                break;
            }
        }
        Ok(final_res)
    }
    fn parse_register_stdout(stdout: &str, register_type: Option<&String>) -> Value {
        if let Some(t) = register_type {
            match t.as_str() {
                "json" => serde_json::from_str(stdout)
                    .unwrap_or_else(|_| Value::String(stdout.trim_end_matches('\n').to_string())),
                "yaml" => serde_yml::from_str(stdout)
                    .unwrap_or_else(|_| Value::String(stdout.trim_end_matches('\n').to_string())),
                _ => Value::String(stdout.trim_end_matches('\n').to_string()),
            }
        } else {
            Value::String(stdout.trim_end_matches('\n').to_string())
        }
    }
}
async fn execute_task_module_with_timeout(
    task: &Task,
    connector: &Connector,
    host_vars: &mut Value,
    timeout_secs: Option<u64>,
) -> Result<TaskResult, TaskModuleError> {
    match timeout_secs {
        Some(secs) if secs > 0 => {
            match tokio::time::timeout(
                std::time::Duration::from_secs(secs),
                execute_task_module(task, connector, host_vars),
            )
            .await
            {
                Ok(r) => r,
                Err(_) => Err(TaskModuleError::Message(format!(
                    "task timed out after {secs} seconds"
                ))),
            }
        }
        _ => execute_task_module(task, connector, host_vars).await,
    }
}
