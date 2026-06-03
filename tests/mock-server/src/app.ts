import { Hono } from "hono";

import { controlRouter } from "./control";
import { accountRouter } from "./routes/account";
import { connectRouter } from "./routes/connect";
import { deployRouter } from "./routes/deploy";
import { deviceRouter } from "./routes/device";
import { passwordRouter } from "./routes/password";
import { quotaRouter } from "./routes/quota";
import { sitesRouter } from "./routes/sites";
import { tokensRouter } from "./routes/tokens";
import { versionsRouter } from "./routes/versions";

export function createApp(): Hono {
  const app = new Hono();

  app.get("/healthz", (c) => c.json({ success: true, data: { status: "ok" } }));

  app.route("/__test", controlRouter);

  const api = new Hono();
  api.route("/account", accountRouter);
  api.route("/quota", quotaRouter);
  api.route("/deploy", deployRouter);
  api.route("/sites", sitesRouter);
  api.route("/sites", passwordRouter);
  api.route("/sites", versionsRouter);
  api.route("/tokens", tokensRouter);
  api.route("/cli/connect", connectRouter);
  api.route("/auth/device", deviceRouter);
  app.route("/api", api);

  return app;
}
