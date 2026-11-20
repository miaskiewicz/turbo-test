// This file is loaded at the top of the test (as Jest's hoisted imports) and already references
// `./x`, so the mock factory must have been registered before this require runs. The factory
// closes over `mockGet`, declared later in the test file.
import { get } from './x';
export const Something = (k: string) => get(k);
