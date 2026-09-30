// Declarations of `book.js`, reached as `yggdryl/book` through the `types`
// condition of the package's `exports` map: the book display the `yggdryl`
// binary embeds, and the spawner of `yggdryl market serve`. `book.js` owns the
// behaviour; this file states its shapes and nothing else.

import type { ChildProcessByStdio } from 'node:child_process'
import type { Readable } from 'node:stream'

/** The absolute folder holding the display's files. */
export declare const assets: string

/** Every file `yggdryl market serve` embeds, in the order the design lists them; frozen. */
export declare const assetFiles: readonly string[]

/** A table the display serves: `name=location`, a bare location, or `{ name, location }`, the location non-empty. */
export type ServeTable = string | { name?: string | null; location: string }

/** What `serveArguments` spells: every option has its default, and a list option takes one item or `null` for none. */
export interface ServeArgumentsOptions {
  /** The tables, first to last; none by default. */
  tables?: ServeTable | readonly ServeTable[] | null
  /** The address the display listens on: `127.0.0.1:0`, a free port. */
  bind?: string
  /** The path the display answers under: `/`. */
  path?: string
  /** FIX bridge logs folded into the first table, one `--capture` each. */
  capture?: string | readonly string[] | null
  /** Further command-line arguments, written verbatim after the rest. */
  args?: string | readonly string[] | null
}

/** What `serve` takes: the command line, then how the process runs. */
export interface ServeOptions extends ServeArgumentsOptions {
  /** The binary: `YGGDRYL_BIN`, else `yggdryl` on the path. */
  bin?: string
  /** The process's environment: `process.env`. */
  env?: NodeJS.ProcessEnv
  /** Ends the process whenever it aborts; `AbortSignal.timeout(ms)` bounds the wait for the endpoint. */
  signal?: AbortSignal
}

/** How the process ended, as its `close` event states it. */
export interface ServeExit {
  code: number | null
  signal: NodeJS.Signals | null
}

/** A running display: resolved once the process has printed its endpoint. */
export interface Serving {
  /** The URL the display answers at. */
  readonly endpoint: string
  /** The child, its stdin ignored and its stdout and stderr piped. */
  readonly process: ChildProcessByStdio<null, Readable, Readable>
  /** End the process, resolving with its exit status once it has closed. */
  close(): Promise<ServeExit>
}

/**
 * The argument vector of `yggdryl market serve`, the binary left out:
 * `market`, `serve`, the tables, `--bind`, `--path`, one `--capture` per
 * log, then `args`.
 *
 * @throws TypeError for a table it cannot spell.
 */
export declare function serveArguments(options?: ServeArgumentsOptions): string[]

/**
 * Start `yggdryl market serve` and resolve once it has printed its endpoint
 * on its first non-empty stdout line. A process that exits first rejects with
 * what it printed as the message: the refusal the command writes on stdout in
 * the endpoint's place - its `✗` line and any line it runs on to - then its
 * stderr, where the argument parser refuses; one that printed neither names
 * its exit status. Rejects with the spawn error when it cannot start, with the
 * line when its first is neither the endpoint, a refusal nor the warning
 * report ahead of one, and with an `AbortError` quoting what it printed once
 * the process `signal` ended has closed. Until it resolves, the process is
 * ended if this one exits.
 *
 * @throws TypeError, before anything is spawned, for a table `serveArguments` cannot spell.
 */
export declare function serve(options?: ServeOptions): Promise<Serving>
