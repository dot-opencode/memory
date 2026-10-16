import { spawn } from "node:child_process"
import { realpath } from "node:fs/promises"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"

import { registerMemoryRuntime } from "./memory-core.js"

const moduleDirectory = await realpath(dirname(fileURLToPath(import.meta.url)))
const memoryCli = resolve(moduleDirectory, "../bin/memory")

// POSIX children get a private process group: never signal the host's group.
// Windows lacks POSIX group signals; its safe fallback only terminates the child.
export function runCommand({ command, args, cwd }, { timeout = 60000, grace = 1000, maxOutput = 1024 * 1024 } = {}) {
  return new Promise((resolveResult, reject) => {
    const grouped = process.platform !== "win32"
    const child = spawn(command, args, {
      cwd,
      shell: false,
      detached: grouped,
      stdio: ["ignore", "pipe", "pipe"],
      env: { ...process.env, GIT_TERMINAL_PROMPT: "0", GIT_EDITOR: "true" },
    })
    const chunks = { stdout: [], stderr: [] }
    const sizes = { stdout: 0, stderr: 0 }
    for (const stream of ["stdout", "stderr"]) {
      child[stream].on("data", (chunk) => {
        const kept = chunk.subarray(0, Math.max(0, maxOutput - sizes[stream]))
        if (kept.length) chunks[stream].push(kept)
        sizes[stream] += kept.length
      })
    }
    let exited = false
    let cleaned = false
    let cleaning = false
    let timedOut = false
    let failure
    let exitCode
    let killTimer
    function signalGroup(signal) {
      if (!child.pid) return
      try {
        if (grouped) process.kill(-child.pid, signal)
        else child.kill(signal)
      } catch (error) {
        if (error.code !== "ESRCH") failure ??= error
      }
    }
    function finish() {
      if (!exited || !cleaned) return
      clearTimeout(deadline)
      clearTimeout(killTimer)
      child.stdout.destroy()
      child.stderr.destroy()
      const output = Object.fromEntries(Object.entries(chunks).map(([key, value]) => [key, Buffer.concat(value)]))
      if (timedOut) reject(Object.assign(new Error("memory backup timed out"), { timedOut: true, ...output }))
      else if (failure) reject(failure)
      else resolveResult({ exitCode, ...output })
    }
    function cleanup() {
      if (cleaning) return
      cleaning = true
      signalGroup("SIGTERM")
      // Do not cancel this when the leader exits: helpers can ignore TERM and
      // retain Git locks even after Bash has gone away.
      killTimer = setTimeout(() => {
        signalGroup("SIGKILL")
        cleaned = true
        finish()
      }, grace)
    }
    const deadline = setTimeout(() => {
      timedOut = true
      cleanup()
    }, timeout)
    child.on("error", (error) => {
      failure = error
      if (!child.pid) {
        exited = true
        cleaned = true
        finish()
      } else cleanup()
    })
    child.on("exit", (code, signal) => {
      exited = true
      exitCode = code
      if (signal && !timedOut) failure ??= new Error(`memory backup terminated by ${signal}`)
      clearTimeout(deadline)
      // Clean up helpers even if the leader exited successfully with redirected
      // stdio; waiting for 'close' alone would not catch these descendants.
      cleanup()
      finish()
    })
  })
}

async function logError(entry) {
  try {
    console.error("[agent-memory]", entry)
  } catch {
    // Logging must not interrupt OpenCode hooks.
  }
}

// A plain V2 plugin object ({ id, setup }). `Plugin.define` from
// @opencode/plugin is an identity function; importing it would make the plugin
// unloadable when installed as a copied skill with no node_modules next to it.
export default {
  id: "agent-memory",
  setup(ctx) {
    return registerMemoryRuntime(ctx, { memoryCli, runBackup: runCommand, logError })
  },
}
