//! Vue 3 Single File Component (.vue) transformer for turbo-test.
//!
//! Parses `.vue` SFC blocks (<script setup>, <script>, <template>, <style>)
//! and compiles them into standard JavaScript / TypeScript modules that oxc
//! and the V8 module runner can execute.

pub mod parser;
pub mod identifiers;
pub mod compiler;

#[cfg(test)]
mod tests;

pub use parser::{parse_sfc, SfcBlock, SfcDescriptor};
pub use compiler::compile_vue_sfc;
