//! Comprehensive unit tests for Vue 3 SFC parsing and compilation.

use std::path::Path;
use super::parser::{extract_attr_value, parse_sfc};
use super::identifiers::{
    component_name_from_path, extract_component_names, extract_setup_identifiers,
};
use super::compiler::compile_vue_sfc;

#[test]
fn test_01_parse_simple_sfc() {
    let sfc = r#"
<template>
  <div class="hello">{{ msg }}</div>
</template>

<script setup lang="ts">
import { ref } from 'vue';
const msg = ref('world');
</script>

<style scoped>
.hello { color: red; }
</style>
"#;
    let desc = parse_sfc(sfc);
    assert!(desc.template.is_some());
    assert!(desc.script_setup.is_some());
    assert_eq!(desc.script_setup.as_ref().unwrap().lang, Some("ts"));
    assert!(desc.script_setup.as_ref().unwrap().is_setup);
    assert!(desc.script.is_none());
}

#[test]
fn test_02_parse_sfc_with_html_comments() {
    let sfc = r#"
<!-- Top level comment -->
<template>
  <!-- Inner comment -->
  <div>Hello</div>
</template>
<!-- Intermediate comment -->
<script setup>
const a = 1;
</script>
"#;
    let desc = parse_sfc(sfc);
    assert!(desc.template.is_some());
    assert!(desc.script_setup.is_some());
    assert!(desc.template.as_ref().unwrap().content.contains("<div>Hello</div>"));
}

#[test]
fn test_03_parse_sfc_with_nested_template_tags() {
    let sfc = r#"
<template>
  <div class="parent">
    <template v-if="ok">
      <span>Nested</span>
    </template>
    <template #slot="{ item }">
      <span>{{ item }}</span>
    </template>
  </div>
</template>
<script setup>
const ok = true;
</script>
"#;
    let desc = parse_sfc(sfc);
    let t = desc.template.expect("must extract full template");
    assert!(t.content.contains("<template v-if=\"ok\">"));
    assert!(t.content.contains("<template #slot=\"{ item }\">"));
    assert!(t.content.contains("</div>"));
}

#[test]
fn test_04_parse_sfc_both_script_and_script_setup() {
    let sfc = r#"
<script lang="ts">
export default {
  inheritAttrs: false,
};
</script>
<script setup lang="ts">
const name = 'SetupBlock';
</script>
<template><div>{{ name }}</div></template>
"#;
    let desc = parse_sfc(sfc);
    assert!(desc.script.is_some());
    assert!(desc.script_setup.is_some());
    assert!(!desc.script.as_ref().unwrap().is_setup);
    assert!(desc.script_setup.as_ref().unwrap().is_setup);
    assert!(desc.script.as_ref().unwrap().content.contains("inheritAttrs: false"));
}

#[test]
fn test_05_parse_sfc_empty_and_garbage() {
    let desc = parse_sfc("");
    assert!(desc.template.is_none());
    assert!(desc.script.is_none());
    assert!(desc.script_setup.is_none());

    let desc2 = parse_sfc("plain text without any tags");
    assert!(desc2.template.is_none());
    assert!(desc2.script.is_none());
    assert!(desc2.script_setup.is_none());

    let desc3 = parse_sfc("<unclosed <template>test");
    assert!(desc3.template.is_none());
}

#[test]
fn test_06_parse_sfc_only_template_no_script() {
    let sfc = "<template><h1>Title</h1></template>";
    let desc = parse_sfc(sfc);
    assert!(desc.template.is_some());
    assert!(desc.script.is_none());
    assert!(desc.script_setup.is_none());
}

#[test]
fn test_07_parse_sfc_only_script_no_template() {
    let sfc = "<script setup>const x = 42;</script>";
    let desc = parse_sfc(sfc);
    assert!(desc.template.is_none());
    assert!(desc.script_setup.is_some());
}

#[test]
fn test_08_extract_attr_value() {
    assert_eq!(extract_attr_value("<script lang=\"ts\">", "lang"), Some("ts"));
    assert_eq!(extract_attr_value("<script lang='js'>", "lang"), Some("js"));
    assert_eq!(extract_attr_value("<script lang=ts>", "lang"), Some("ts"));
    assert_eq!(extract_attr_value("<script setup>", "setup"), None);
    assert_eq!(extract_attr_value("<script setup lang=\"ts\">", "lang"), Some("ts"));
}

#[test]
fn test_09_component_name_from_path() {
    assert_eq!(component_name_from_path(Path::new("MyComponent.vue")), "MyComponent");
    assert_eq!(component_name_from_path(Path::new("path/to/SettingsNavRail.vue")), "SettingsNavRail");
    assert_eq!(component_name_from_path(Path::new("")), "AnonymousComponent");
}

#[test]
fn test_10_extract_setup_identifiers_basic() {
    let code = r#"
const a = 1;
let b = 'hello';
var c = true;
function myFunc() { return 123; }
"#;
    let ids = extract_setup_identifiers(code);
    assert!(ids.contains(&"a".to_string()));
    assert!(ids.contains(&"b".to_string()));
    assert!(ids.contains(&"c".to_string()));
    assert!(ids.contains(&"myFunc".to_string()));
}

#[test]
fn test_11_extract_setup_identifiers_destructuring() {
    let code = r#"
const { foo, bar: renamedBar, baz = 100 } = useComposable();
const [firstItem, secondItem] = tuple;
"#;
    let ids = extract_setup_identifiers(code);
    assert!(ids.contains(&"foo".to_string()));
    assert!(ids.contains(&"renamedBar".to_string()));
    assert!(ids.contains(&"baz".to_string()));
    assert!(ids.contains(&"firstItem".to_string()));
    assert!(ids.contains(&"secondItem".to_string()));
}

#[test]
fn test_12_extract_setup_identifiers_multiline_destructuring() {
    let code = r#"
const {
  showSidebar,
  activeChatId,
  isGenerating,
} = useAiMainChatView(props);
"#;
    let ids = extract_setup_identifiers(code);
    assert!(ids.contains(&"showSidebar".to_string()));
    assert!(ids.contains(&"activeChatId".to_string()));
    assert!(ids.contains(&"isGenerating".to_string()));
}

#[test]
fn test_13_extract_component_names_pascal_case() {
    let imports = vec![
        "import SettingsNavRail from './SettingsNavRail.vue';",
        "import { AiHeader, AiChatHistory, formatTime } from '../index';",
        "import type { AiMainChatViewProps } from './model';",
    ];
    let comps = extract_component_names(&imports);
    assert!(comps.contains(&"SettingsNavRail".to_string()));
    assert!(comps.contains(&"AiHeader".to_string()));
    assert!(comps.contains(&"AiChatHistory".to_string()));
    // lowercase utilities must not be treated as components
    assert!(!comps.contains(&"formatTime".to_string()));
    // type-only imports must be excluded
    assert!(!comps.contains(&"AiMainChatViewProps".to_string()));
}

#[test]
fn test_14_compile_sfc_basic_structure() {
    let sfc = r#"
<template>
  <div>Test</div>
</template>
<script setup lang="ts">
import { ref } from 'vue';
const x = ref(1);
</script>
"#;
    let compiled = compile_vue_sfc(Path::new("MyComp.vue"), sfc).unwrap();
    assert!(compiled.contains("export default _sfc_main;"));
    assert!(compiled.contains("import { ref } from 'vue'"));
    assert!(compiled.contains("name: \"MyComp\""));
    assert!(compiled.contains("_sfc_bindings.x = x"));
    assert!(compiled.contains("components: Object.assign({}, _sfc_components"));
}

#[test]
fn test_15_compile_sfc_props_and_with_defaults_shim() {
    let sfc = r#"
<template><div>{{ pageSize }}</div></template>
<script setup lang="ts">
interface Props { pageSize?: string; isLandscape?: boolean; }
const props = withDefaults(defineProps<Props>(), {
  pageSize: 'A4',
  isLandscape: false,
});
</script>
"#;
    let compiled = compile_vue_sfc(Path::new("WordRuler.vue"), sfc).unwrap();
    // withDefaults shim must be present
    assert!(compiled.contains("const withDefaults = (props, defaults) => {"));
    // __props proxy must handle attrs fallback
    assert!(compiled.contains("const __props = new Proxy(__props_raw || {}, {"));
    assert!(compiled.contains("defineProps = (_opts) => __props"));
}

#[test]
fn test_16_compile_sfc_hoists_types_and_multiline_imports() {
    let sfc = r#"
<template><div>Test</div></template>
<script setup lang="ts">
import {
  ref,
  computed
} from 'vue';
export type { PaletteCommand };
export interface MyProps { id: string }
export enum Direction { Up = 'UP', Down = 'DOWN' }
const emit = defineEmits<any>();
const x = ref(1);
</script>
"#;
    let compiled = compile_vue_sfc(Path::new("CommandPalette.vue"), sfc).unwrap();
    assert!(compiled.contains("export type { PaletteCommand }"));
    assert!(compiled.contains("export interface MyProps"));
    assert!(compiled.contains("export enum Direction"));
    assert!(compiled.contains("computed\n} from 'vue'"));
    assert!(compiled.contains("setup(__props_raw, { attrs: __attrs, emit: __emit, expose: __expose })"));

    let setup_pos = compiled.find("setup(__props_raw").unwrap();
    let type_pos = compiled.find("export type { PaletteCommand }").unwrap();
    let iface_pos = compiled.find("export interface MyProps").unwrap();
    let enum_pos = compiled.find("export enum Direction").unwrap();

    assert!(type_pos < setup_pos, "export type must be hoisted outside setup()");
    assert!(iface_pos < setup_pos, "export interface must be hoisted outside setup()");
    assert!(enum_pos < setup_pos, "export enum must be hoisted outside setup()");
}

#[test]
fn test_17_compile_sfc_escapes_backticks_and_interpolations() {
    let sfc = r#"
<template>
  <div :title="`raw backtick`">Hello ${notInterpolated} \ backslash</div>
</template>
<script setup>
const msg = 'hi';
</script>
"#;
    let compiled = compile_vue_sfc(Path::new("EscapedComp.vue"), sfc).unwrap();
    assert!(compiled.contains(r"\`raw backtick\`"));
    assert!(compiled.contains(r"\${notInterpolated}"));
    assert!(compiled.contains(r"\\ backslash"));
}

#[test]
fn test_18_compile_sfc_normal_script_merged() {
    let sfc = r#"
<script lang="ts">
export default {
  name: 'CustomName',
  methods: {
    hello() { return 'hi'; }
  }
};
</script>
<template><div>{{ count }}</div></template>
<script setup lang="ts">
import { ref } from 'vue';
const count = ref(0);
</script>
"#;
    let compiled = compile_vue_sfc(Path::new("MergedComp.vue"), sfc).unwrap();
    assert!(compiled.contains("const _sfc_script = {"));
    assert!(compiled.contains("hello() { return 'hi'; }"));
    assert!(compiled.contains("..._sfc_script"));
    assert!(compiled.contains("_sfc_bindings.count = count"));
}

#[test]
fn test_19_compile_sfc_props_through_attrs_on_windows_path() {
    let sfc = r#"
<template><div>{{ panel.showSidebar }}</div></template>
<script setup lang="ts">
const props = defineProps<{ panel: any }>();
const { showSidebar } = props.panel;
</script>
"#;
    let compiled = compile_vue_sfc(Path::new("C:\\WorldWiki\\AiMainChatView.vue"), sfc).unwrap();
    assert!(compiled.contains("__props_raw || {}"));
    assert!(compiled.contains("__attrs && __attrs[k]"));
    assert!(compiled.contains("defineProps = (_opts) => __props"));
    assert!(compiled.contains("name: \"AiMainChatView\""));
}

#[test]
fn test_20_compile_sfc_multiple_script_setup_blocks() {
    let sfc = r#"
<template><div>Double Setup</div></template>
<script setup lang="ts">
const a = 1;
</script>
<script setup lang="ts">
const b = 2;
</script>
"#;
    let result = compile_vue_sfc(Path::new("DoubleSetup.vue"), sfc);
    assert!(result.is_err(), "multiple script setup blocks must return Err");
    let err = result.unwrap_err();
    assert!(err.contains("multiple <script setup> blocks"));
}

#[test]
fn test_21_compile_sfc_define_props_with_defaults_and_required() {
    let sfc = r#"
<template><div>{{ a }} - {{ b }}</div></template>
<script setup lang="ts">
interface MyProps { a: string; b?: number; }
const props = withDefaults(defineProps<MyProps>(), { b: 0 });
</script>
"#;
    let compiled = compile_vue_sfc(Path::new("PropsWithDefaults.vue"), sfc).unwrap();
    assert!(compiled.contains("withDefaults = (props, defaults) => {"));
    assert!(compiled.contains("const res = Object.assign({}, defaults);"));
    assert!(compiled.contains("if (props[k] !== undefined) res[k] = props[k];"));
}

#[test]
fn test_22_compile_sfc_emits_with_types() {
    let sfc = r#"
<template><button @click="emit('change', 'val')">Click</button></template>
<script setup lang="ts">
const emit = defineEmits<{ (e: 'change', v: string): void }>();
</script>
"#;
    let compiled = compile_vue_sfc(Path::new("EmitsComponent.vue"), sfc).unwrap();
    // Parameter is renamed to __emit so local `const emit = defineEmits(...)` does not clash
    assert!(compiled.contains("emit: __emit"));
    assert!(compiled.contains("const defineEmits = (_opts) => __emit"));
    assert!(compiled.contains("const emit = defineEmits"));
}

#[test]
fn test_23_parse_sfc_template_with_script_tag_in_string() {
    let sfc = r#"
<template>
  <div>{{ "<script>alert(1)</script>" }}</div>
</template>
<script setup>
const msg = "ok";
</script>
"#;
    let desc = parse_sfc(sfc);
    let t = desc.template.expect("template must be parsed");
    assert!(t.content.contains(r#"<script>alert(1)</script>"#));
    let s = desc.script_setup.expect("script setup must be parsed");
    assert!(s.content.contains("const msg = \"ok\";"));
}

#[test]
fn test_24_parse_sfc_style_with_template_tag_in_css() {
    let sfc = r#"
<style>
.x::before { content: "<template>"; }
</style>
<template>
  <div class="x">Real Template</div>
</template>
<script setup>
const ready = true;
</script>
"#;
    let desc = parse_sfc(sfc);
    let t = desc.template.expect("real template must be extracted");
    assert!(t.content.contains("Real Template"));
    assert!(!t.content.contains(".x::before"));
}

#[test]
fn test_25_compile_sfc_imports_hoisted_before_setup() {
    let sfc = r#"
<template><div>{{ val }}</div></template>
<script setup lang="ts">
import {
  ref,
  computed
} from 'vue';
import type { SomeType } from './types';
export type LocalType = string;
export interface LocalIface { id: number; }
const val = ref('hello');
</script>
"#;
    let compiled = compile_vue_sfc(Path::new("HoistingTest.vue"), sfc).unwrap();
    let import_idx = compiled.find("from 'vue'").unwrap();
    let type_idx = compiled.find("export type LocalType").unwrap();
    let iface_idx = compiled.find("export interface LocalIface").unwrap();
    let setup_idx = compiled.find("setup(__props_raw").unwrap();

    assert!(import_idx < type_idx, "imports must be before hoisted types");
    assert!(type_idx < setup_idx, "hoisted types must be before setup()");
    assert!(iface_idx < setup_idx, "hoisted interfaces must be before setup()");
}

#[test]
fn test_26_compile_sfc_components_registered() {
    let sfc = r#"
<template>
  <SettingsNavRail />
  <ThemeSwitcher />
</template>
<script setup lang="ts">
import SettingsNavRail from './SettingsNavRail.vue';
import { ThemeSwitcher, helperUtil } from '../theme';
</script>
"#;
    let compiled = compile_vue_sfc(Path::new("Settings.vue"), sfc).unwrap();
    assert!(compiled.contains("_sfc_components.SettingsNavRail = SettingsNavRail"));
    assert!(compiled.contains("_sfc_components.ThemeSwitcher = ThemeSwitcher"));
    assert!(!compiled.contains("_sfc_components.helperUtil"), "camelCase util must not be registered as component");
    assert!(compiled.contains("components: Object.assign({}, _sfc_components"));
}

#[test]
fn test_27_compile_sfc_setup_bindings_exposed() {
    let sfc = r#"
<template><div v-if="showSidebar">{{ activeChatId }}</div></template>
<script setup lang="ts">
const {
  showSidebar,
  activeChatId,
} = useAiChat();
</script>
"#;
    let compiled = compile_vue_sfc(Path::new("BindingsTest.vue"), sfc).unwrap();
    assert!(compiled.contains("_sfc_bindings.showSidebar = showSidebar"));
    assert!(compiled.contains("_sfc_bindings.activeChatId = activeChatId"));
    assert!(compiled.contains("return _sfc_bindings"));
}

#[test]
fn test_28_compile_sfc_windows_paths() {
    assert_eq!(component_name_from_path(Path::new("C:\\WorldWiki\\src\\Bar.vue")), "Bar");
    assert_eq!(component_name_from_path(Path::new("D:\\foo\\bar\\SettingsNavRail.vue")), "SettingsNavRail");
    assert_eq!(component_name_from_path(Path::new("\\\\server\\share\\MyComp.vue")), "MyComp");
}

#[test]
fn test_29_compile_sfc_cache_key_invalidation() {
    let build_id = env!("TT_BUILD_ID");
    assert!(!build_id.is_empty(), "TT_BUILD_ID must be populated by build.rs");
    assert_eq!(build_id.len(), 16, "TT_BUILD_ID must be a 16-hex-digit hash");
}

#[test]
fn test_30_compile_sfc_wasm_import() {
    let sfc = r#"
<template><div>{{ isReady }}</div></template>
<script setup lang="ts">
import init, { WasmCalendarEngine, initSync } from './worldwiki_calendar_wasm.js';
import type { CalendarAuditReport } from './types';
const isReady = true;
</script>
"#;
    let compiled = compile_vue_sfc(Path::new("WasmConsumer.vue"), sfc).unwrap();
    assert!(compiled.contains("import init, { WasmCalendarEngine, initSync } from './worldwiki_calendar_wasm.js'"));
    let import_pos = compiled.find("worldwiki_calendar_wasm").unwrap();
    let setup_pos = compiled.find("setup(__props_raw").unwrap();
    assert!(import_pos < setup_pos, "WASM import must be hoisted before setup()");
    assert!(compiled.contains("_sfc_bindings.isReady = isReady"));
}

#[test]
fn test_31_compile_sfc_runtime_compiler_from_vue_esm_bundler() {
    let sfc = r#"
<template>
  <div class="settings-layout">
    <SettingsNavRail :active-tab="activeTab" />
  </div>
</template>
<script setup lang="ts">
import SettingsNavRail from './Settings/SettingsNavRail.vue';
const activeTab = 'appearance';
</script>
"#;
    let compiled = compile_vue_sfc(Path::new("Settings.vue"), sfc).unwrap();
    // Must import the full compiler bundle, not the runtime-only stub that throws/warns
    assert!(
        compiled.contains("import { compile as _compile } from 'vue/dist/vue.esm-bundler.js'"),
        "Vue SFC must import compile from vue/dist/vue.esm-bundler.js so runtime template compiler is registered"
    );
    assert!(compiled.contains("_sfc_render = _compile(_sfc_template)"));
}

#[test]
fn test_32_compile_sfc_child_components_direct_reference_no_typeof_guard() {
    let sfc = r#"
<template>
  <SettingsNavRail />
  <ThemeSwitcher />
</template>
<script setup lang="ts">
import SettingsNavRail from './SettingsNavRail.vue';
import { ThemeSwitcher } from './ThemeSwitcher';
</script>
"#;
    let compiled = compile_vue_sfc(Path::new("Settings.vue"), sfc).unwrap();
    // Must be direct references so oxc/esm_cjs rewrites them to import_SettingsNavRail.default
    // (typeof SettingsNavRail !== 'undefined' prevents the CJS import rewrite)
    assert!(compiled.contains("try { _sfc_components.SettingsNavRail = SettingsNavRail; } catch (_e) {}"));
    assert!(compiled.contains("try { _sfc_components.ThemeSwitcher = ThemeSwitcher; } catch (_e) {}"));
    assert!(compiled.contains("try { _sfc_bindings.SettingsNavRail = SettingsNavRail; } catch (_e) {}"));
    assert!(compiled.contains("try { _sfc_bindings.ThemeSwitcher = ThemeSwitcher; } catch (_e) {}"));
}

#[test]
fn test_33_compile_sfc_with_defaults_and_attrs_fallback() {
    let sfc = r#"
<template>
  <div>{{ panel.showSidebar }}</div>
  <div>{{ pageSize }}</div>
</template>
<script setup lang="ts">
interface Props {
  panel?: { showSidebar: boolean };
  pageSize?: number;
}
const props = withDefaults(defineProps<Props>(), {
  pageSize: 20
});
</script>
"#;
    let compiled = compile_vue_sfc(Path::new("WithDefaultsTest.vue"), sfc).unwrap();
    assert!(compiled.contains("withDefaults = (props, defaults) =>"));
    assert!(compiled.contains("const __props = new Proxy(__props_raw || {}"));
    assert!(compiled.contains("__attrs && __attrs[k]"));
}

#[test]
fn test_34_compile_sfc_strips_template_type_assertions_and_binds_define_model() {
    let sfc = r#"
<template>
  <input :value="val" @input="e => update((e.target as HTMLInputElement).value)">
  <p>{{ (name as string) }}</p>
</template>
<script setup lang="ts">
import type { AppSettings } from './types';
const settings = defineModel<AppSettings>('settings', { required: true });
const val = 'hello';
const update = (v: string) => {};
</script>
"#;
    let compiled = compile_vue_sfc(Path::new("TypeCastTest.vue"), sfc).unwrap();
    assert!(compiled.contains("(e.target).value"), "Template must strip `as HTMLInputElement`");
    assert!(compiled.contains("(name)"), "Template must strip `as string`");
    assert!(!compiled.contains("as HTMLInputElement"));
    assert!(compiled.contains("const defineModel = (name, _opts) =>"));
    assert!(compiled.contains("return __props[propName]"));
}



