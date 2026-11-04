//! Vue SFC Code Generator.
//!
//! Assembles `<script setup>`, `<script>` and `<template>` into an executable
//! Vue 3 component module exportable to oxc / V8.

use std::path::Path;
use super::parser::parse_sfc;
use super::identifiers::{component_name_from_path, extract_component_names, extract_setup_identifiers};

fn strip_template_type_assertions(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(idx) = rest.find(" as ") {
        out.push_str(&rest[..idx]);
        let after = &rest[idx + 4..];
        let mut chars = after.char_indices();
        let mut id_end = 0;
        let mut has_id = false;
        while let Some((c_idx, ch)) = chars.next() {
            if ch.is_alphanumeric() || ch == '_' || ch == '$' {
                has_id = true;
                id_end = c_idx + ch.len_utf8();
            } else {
                break;
            }
        }
        if has_id {
            let tail = after[id_end..].trim_start();
            if tail.starts_with(')') || tail.starts_with(',') || tail.starts_with(';') {
                rest = &after[id_end..];
                continue;
            }
        }
        out.push_str(" as ");
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Transforms Vue SFC source into executable TypeScript/JavaScript.
pub fn compile_vue_sfc(path: &Path, src: &str) -> Result<String, String> {
    let descriptor = parse_sfc(src);
    if descriptor.has_multiple_script_setup {
        return Err("A Single File Component cannot contain multiple <script setup> blocks".to_string());
    }
    let comp_name = component_name_from_path(path);

    let mut out = String::with_capacity(src.len() + 1024);

    // 1. Preamble & Vue imports
    out.push_str("/* turbo-test: compiled Vue SFC */\n");
    out.push_str("import { defineComponent as _defineComponent } from 'vue';\n");
    out.push_str("import { compile as _compile } from 'vue/dist/vue.esm-bundler.js';\n");

    let mut imports = Vec::new();
    let mut setup_body = String::new();
    let mut normal_script_body = String::new();

    let setup_ids = descriptor
        .script_setup
        .as_ref()
        .map(|s| extract_setup_identifiers(s.content))
        .unwrap_or_default();

    // 2. Process `<script setup>` if present
    if let Some(setup_block) = descriptor.script_setup {
        let mut in_import = false;
        let mut in_hoisted_export = false;
        let mut export_brace_depth: i32 = 0;

        for line in setup_block.content.lines() {
            let trimmed = line.trim();

            if in_import {
                imports.push(line);
                if trimmed.contains("from ")
                    || trimmed.ends_with("';")
                    || trimmed.ends_with("\";")
                    || (trimmed.ends_with(';') && !trimmed.contains('{'))
                {
                    in_import = false;
                }
            } else if trimmed.starts_with("import ") && !trimmed.contains("import(") {
                imports.push(line);
                if !trimmed.contains("from ") && !trimmed.ends_with(';') {
                    in_import = true;
                }
            } else if in_hoisted_export {
                normal_script_body.push_str(line);
                normal_script_body.push('\n');
                export_brace_depth += line.matches('{').count() as i32;
                export_brace_depth -= line.matches('}').count() as i32;
                if export_brace_depth <= 0 && (trimmed.ends_with(';') || trimmed.ends_with('}')) {
                    in_hoisted_export = false;
                }
            } else if trimmed.starts_with("export type ")
                || trimmed.starts_with("export interface ")
                || trimmed.starts_with("export enum ")
                || trimmed.starts_with("export {")
                || trimmed.starts_with("type ")
                || trimmed.starts_with("interface ")
            {
                normal_script_body.push_str(line);
                normal_script_body.push('\n');
                export_brace_depth = line.matches('{').count() as i32 - line.matches('}').count() as i32;
                if export_brace_depth > 0
                    || (!trimmed.ends_with(';')
                        && !trimmed.ends_with('}')
                        && (trimmed.starts_with("export type")
                            || trimmed.starts_with("export {")
                            || trimmed.starts_with("type ")))
                {
                    in_hoisted_export = true;
                }
            } else {
                setup_body.push_str(line);
                setup_body.push('\n');
            }
        }
    }

    // 3. Process normal `<script>` if present
    let mut has_export_default = false;
    if let Some(script_block) = descriptor.script {
        let mut in_import = false;
        for line in script_block.content.lines() {
            let trimmed = line.trim();
            if in_import {
                imports.push(line);
                if trimmed.contains("from ")
                    || trimmed.ends_with("';")
                    || trimmed.ends_with("\";")
                    || (trimmed.ends_with(';') && !trimmed.contains('{'))
                {
                    in_import = false;
                }
            } else if trimmed.starts_with("import ") && !trimmed.contains("import(") {
                imports.push(line);
                if !trimmed.contains("from ") && !trimmed.ends_with(';') {
                    in_import = true;
                }
            } else if trimmed.starts_with("export default") {
                has_export_default = true;
                let replaced = line.replacen("export default", "const _sfc_script =", 1);
                normal_script_body.push_str(&replaced);
                normal_script_body.push('\n');
            } else {
                normal_script_body.push_str(line);
                normal_script_body.push('\n');
            }
        }
    }

    // 4. Output hoisted imports & component registration
    let component_names = extract_component_names(&imports);
    for imp in imports {
        out.push_str(imp);
        out.push('\n');
    }
    out.push('\n');

    out.push_str("const _sfc_components = {};\n");
    for comp in &component_names {
        out.push_str(&format!(
            "try {{ _sfc_components.{comp} = {comp}; }} catch (_e) {{}}\n"
        ));
    }
    out.push('\n');

    // 5. Output normal script body
    if !normal_script_body.is_empty() {
        out.push_str("/* Normal script */\n");
        out.push_str(&normal_script_body);
        out.push('\n');
    }
    if !has_export_default {
        out.push_str("const _sfc_script = {};\n");
    }

    // 6. Template string representation
    let raw_template = descriptor.template.map(|t| t.content).unwrap_or("");
    let sanitized_template = strip_template_type_assertions(raw_template);
    let escaped_template = sanitized_template
        .replace('\\', "\\\\")
        .replace('`', "\\`")
        .replace("${", "\\${");

    // 7. Compile template at runtime via `vue.compile` if available
    out.push_str(&format!("const _sfc_template = `{}`;\n", escaped_template));
    out.push_str("let _sfc_render = null;\n");
    out.push_str("if (typeof _compile === 'function' && _sfc_template.length > 0) {\n");
    out.push_str("  try {\n");
    out.push_str("    _sfc_render = _compile(_sfc_template);\n");
    out.push_str("  } catch (_e) {}\n");
    out.push_str("}\n\n");

    // 8. Component definition
    out.push_str("const _sfc_main = _defineComponent({\n");
    out.push_str("  ..._sfc_script,\n");
    out.push_str(&format!("  name: {:?},\n", comp_name));
    out.push_str("  components: Object.assign({}, _sfc_components, _sfc_script.components || {}),\n");
    out.push_str("  template: _sfc_template,\n");
    out.push_str("  ...(_sfc_render ? { render: _sfc_render } : {}),\n");

    if !setup_body.is_empty() {
        out.push_str("  setup(__props_raw, { attrs: __attrs, emit: __emit, expose: __expose }) {\n");
        out.push_str("    const __toCamel = (s) => typeof s === 'string' ? s.replace(/-([a-z])/g, (_, c) => c.toUpperCase()) : s;\n");
        out.push_str("    const __toKebab = (s) => typeof s === 'string' ? s.replace(/[A-Z]/g, m => '-' + m.toLowerCase()) : s;\n");
        out.push_str("    const __props = new Proxy(__props_raw || {}, {\n");
        out.push_str("      get: (t, k) => (k in t && t[k] !== undefined ? t[k] : ((__attrs && __attrs[k]) !== undefined ? __attrs[k] : ((__attrs && __attrs[__toKebab(k)]) !== undefined ? __attrs[__toKebab(k)] : (__attrs && __attrs[__toCamel(k)])))),\n");
        out.push_str("      has: (t, k) => (k in t || (__attrs && (k in __attrs || __toKebab(k) in __attrs || __toCamel(k) in __attrs))),\n");
        out.push_str("      ownKeys: (t) => Array.from(new Set([...Reflect.ownKeys(t), ...(__attrs ? Reflect.ownKeys(__attrs) : [])])),\n");
        out.push_str("      getOwnPropertyDescriptor: (t, k) => Object.getOwnPropertyDescriptor(t, k) || (__attrs && Object.getOwnPropertyDescriptor(__attrs, k))\n");
        out.push_str("    });\n\n");
        // Compiler macro shims so they don't fail at runtime if invoked
        out.push_str("    const defineProps = (_opts) => __props;\n");
        out.push_str("    const defineEmits = (_opts) => __emit;\n");
        out.push_str("    const defineExpose = (exp) => { if (typeof __expose === 'function') __expose(exp); return exp; };\n");
        out.push_str("    const withDefaults = (props, defaults) => {\n");
        out.push_str("      const res = Object.assign({}, defaults);\n");
        out.push_str("      if (props) {\n");
        out.push_str("        for (const k in props) {\n");
        out.push_str("          if (props[k] !== undefined) res[k] = props[k];\n");
        out.push_str("        }\n");
        out.push_str("      }\n");
        out.push_str("      return res;\n");
        out.push_str("    };\n");
        out.push_str("    const defineOptions = (_opts) => {};\n");
        out.push_str("    const defineSlots = () => ({});\n");
        out.push_str("    const defineModel = (name, _opts) => {\n");
        out.push_str("      const propName = typeof name === 'string' ? name : 'modelValue';\n");
        out.push_str("      const target = {\n");
        out.push_str("        __v_isRef: true,\n");
        out.push_str("        get value() { return __props[propName]; },\n");
        out.push_str("        set value(v) { if (typeof __emit === 'function') __emit('update:' + propName, v); }\n");
        out.push_str("      };\n");
        out.push_str("      return new Proxy(target, {\n");
        out.push_str("        get(t, k) {\n");
        out.push_str("          if (k in t) return t[k];\n");
        out.push_str("          const val = t.value;\n");
        out.push_str("          if (val && typeof val === 'object' && k in val) return val[k];\n");
        out.push_str("          return undefined;\n");
        out.push_str("        },\n");
        out.push_str("        set(t, k, v) {\n");
        out.push_str("          if (k === 'value') { t.value = v; return true; }\n");
        out.push_str("          const val = t.value;\n");
        out.push_str("          if (val && typeof val === 'object') { val[k] = v; return true; }\n");
        out.push_str("          return false;\n");
        out.push_str("        }\n");
        out.push_str("      });\n");
        out.push_str("    };\n\n");
        out.push_str(&setup_body);

        out.push_str("\n\n    const _sfc_bindings = Object.assign({}, __props);\n");
        for comp in &component_names {
            out.push_str(&format!(
                "    try {{ _sfc_bindings.{comp} = {comp}; }} catch (_e) {{}}\n"
            ));
        }
        for id in &setup_ids {
            out.push_str(&format!(
                "    try {{ _sfc_bindings.{id} = {id}; }} catch (_e) {{}}\n"
            ));
        }
        out.push_str("    if (__props_raw) {\n");
        out.push_str("      for (const k in __props_raw) {\n");
        out.push_str("        const c = __toCamel(k);\n");
        out.push_str("        if (!(c in _sfc_bindings)) _sfc_bindings[c] = __props_raw[k];\n");
        out.push_str("        const kb = __toKebab(k);\n");
        out.push_str("        if (!(kb in _sfc_bindings)) _sfc_bindings[kb] = __props_raw[k];\n");
        out.push_str("      }\n");
        out.push_str("    }\n");
        out.push_str("    if (__attrs) {\n");
        out.push_str("      for (const k in __attrs) {\n");
        out.push_str("        const c = __toCamel(k);\n");
        out.push_str("        if (!(c in _sfc_bindings)) _sfc_bindings[c] = __attrs[k];\n");
        out.push_str("        const kb = __toKebab(k);\n");
        out.push_str("        if (!(kb in _sfc_bindings)) _sfc_bindings[kb] = __attrs[k];\n");
        out.push_str("      }\n");
        out.push_str("    }\n");
        out.push_str("    return _sfc_bindings;\n  },\n");
    }

    out.push_str("});\n\n");
    out.push_str("export default _sfc_main;\n");

    Ok(out)
}

