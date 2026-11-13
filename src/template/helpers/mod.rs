//! Built-in helper functions registered on [`gtmpl::Template`] instances.

mod collections;
mod math;
mod misc;
mod network;
mod serde;
mod string;

/// Registers all built-in helper functions onto the given `Go` template instance.
pub fn register_functions(t: &mut gtmpl::Template) {
    t.add_func("toJson", serde::to_json_fn);
    t.add_func("toYaml", serde::to_yaml_fn);
    t.add_func("fromYaml", serde::from_yaml_fn);
    t.add_func("has", collections::has_fn);
    t.add_func("ternary", collections::ternary_fn);
    t.add_func("trimPrefix", string::trim_prefix_fn);
    t.add_func("trimSuffix", string::trim_suffix_fn);
    t.add_func("join", string::join_fn);
    t.add_func("append", collections::append_fn);
    t.add_func("cat", string::cat_fn);
    t.add_func("regexMatch", string::regex_match_fn);
    t.add_func("regexReplaceAll", string::regex_replace_all_fn);
    t.add_func("randInt", math::rand_int_fn);
    t.add_func("add", math::add_fn);
    t.add_func("sub", math::sub_fn);
    t.add_func("int", math::int_fn);
    t.add_func("len", math::len_fn);
    t.add_func("slice", collections::slice_fn);
    t.add_func("first", collections::first_fn);
    t.add_func("index", collections::index_fn);
    t.add_func("default", collections::default_fn);
    t.add_func("eq", collections::eq_fn);
    t.add_func("ne", collections::ne_fn);
    t.add_func("empty", collections::empty_fn);
    t.add_func("not", collections::not_fn);
    t.add_func("indent", string::indent_fn);
    t.add_func("splitList", string::split_list_fn);
    t.add_func("b64enc", string::b64enc_fn);
    t.add_func("ipInCIDR", network::ip_in_cidr_fn);
    t.add_func("generateIPsFromCIDR", network::generate_ips_from_cidr_fn);
    t.add_func("ipFamily", network::ip_family_fn);
    t.add_func("toLowerByteUnit", collections::to_lower_byte_unit_fn);
    t.add_func(
        "mapToNamedStringArgs",
        collections::map_to_named_string_args_fn,
    );
    t.add_func("pow", math::pow_fn);
    t.add_func("subtractList", collections::subtract_list_fn);
    t.add_func("fileExists", misc::file_exists_fn);
    t.add_func("unquote", string::unquote_fn);
    t.add_func("getStringSlice", collections::get_string_slice_fn);
    t.add_func("semverCompare", misc::semver_compare_fn);
    t.add_func("tpl", misc::tpl_fn);
    t.add_func("include", misc::include_fn);
}
