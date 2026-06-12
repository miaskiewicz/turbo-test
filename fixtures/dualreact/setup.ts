// The setup file pulls the peer-React ESM dep into the run_setup_file bundle
// (esbuild_bundle_full). Expose the SETUP-bundle's copy of the dep + its hook so the
// test can exercise exactly the boundary that breaks in payroll-app (a setup-mock factory
// rendering a node_modules ESM component bundled with its own baked React).
import { depReact, depUseRef } from 'dual-react-dep';
(globalThis as any).__depReactSetup = depReact;
(globalThis as any).__depUseRefSetup = depUseRef;
