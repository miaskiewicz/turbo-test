// Setup files load through the ESM graph (not the CJS module-runner), where a named import from
// a CJS target needs real export names.
import { blockFields } from '@fx/pkg/sub';
(globalThis as any).__setupBlockFields = blockFields;
