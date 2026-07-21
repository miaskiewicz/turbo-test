// Setup file (runs once per reused worker). Importing the package triggers its top-level
// expect.extend side-effect — exactly how a real suite pulls in @testing-library/jest-dom.
import 'jest-dom-ish';
