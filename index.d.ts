export interface RunOptions {
  /** Worker count (default: CPU cores). */
  jobs?: number;
  /** Reporter: "json" for a vitest-ish JSON summary. */
  reporter?: string;
  /** Shard, e.g. "1/4". */
  shard?: string;
  /** Extra env vars (e.g. { TURBO_REUSE_ISOLATE: "1" }). */
  env?: Record<string, string>;
}

/** Run turbo-test over the given test files. Returns the process exit status. */
export function run(files: string[], opts?: RunOptions): { status: number };

/** Resolve the native binary path for this platform, or null. */
export function binaryPath(): string | null;
