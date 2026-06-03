import { createApp } from "./app";

interface ParsedArgs {
  port: number;
}

function parseArgs(argv: string[]): ParsedArgs {
  let port = 0;
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--port") {
      const next = argv[i + 1];
      if (next === undefined) {
        throw new Error("--port requires a value");
      }
      port = parseIntStrict(next);
      i += 1;
    } else if (arg !== undefined && arg.startsWith("--port=")) {
      port = parseIntStrict(arg.slice("--port=".length));
    } else if (arg === "--help" || arg === "-h") {
      process.stderr.write("usage: bun run src/main.ts [--port <n>]\n");
      process.exit(0);
    } else {
      throw new Error(`unknown argument: ${arg}`);
    }
  }
  return { port };
}

function parseIntStrict(value: string): number {
  if (!/^\d+$/.test(value)) {
    throw new Error(`invalid port value: ${value}`);
  }
  const parsed = Number.parseInt(value, 10);
  if (!Number.isFinite(parsed) || parsed < 0 || parsed > 65535) {
    throw new Error(`port out of range: ${value}`);
  }
  return parsed;
}

function main(): void {
  let args: ParsedArgs;
  try {
    args = parseArgs(process.argv.slice(2));
  } catch (err) {
    process.stderr.write(`${(err as Error).message}\n`);
    process.exit(2);
  }

  const app = createApp();

  const server = Bun.serve({
    port: args.port,
    hostname: "127.0.0.1",
    fetch: app.fetch,
  });

  process.stdout.write(`LISTENING http://127.0.0.1:${server.port}\n`);

  const shutdown = (signal: string): void => {
    process.stderr.write(`received ${signal}, shutting down\n`);
    server.stop(true);
    process.exit(0);
  };

  process.on("SIGTERM", () => shutdown("SIGTERM"));
  process.on("SIGINT", () => shutdown("SIGINT"));

  // Auto-exit when the spawning parent closes our stdin. Survives every
  // exit mode the parent can take — graceful exit, panic, even SIGKILL —
  // because the kernel closes the write end of the pipe when the parent
  // dies, regardless of whether any signal was delivered. This is the
  // single mechanism that prevents orphan mock servers from piling up on
  // shared dev hosts; the SIGTERM/SIGINT handlers above only cover the
  // explicit-shutdown path.
  void (async () => {
    try {
      for await (const _ of Bun.stdin.stream()) {
        // Bytes on stdin are ignored — the channel is used for liveness,
        // not control. Any payload accidentally arriving is dropped.
      }
    } catch {
      // stdin read errors collapse to the same "parent gone" signal as
      // an orderly EOF; either way the mock should exit.
    }
    shutdown("STDIN_EOF");
  })();
}

main();
