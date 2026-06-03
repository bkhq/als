import { Hono } from "hono";

import { errorResponse, ok } from "./envelope";
import {
  state,
  type ConnectSession,
  type InjectedFailure,
  type Site,
  type Token,
  type User,
} from "./state";

interface SeedBody {
  sites?: Site[];
  tokens?: Token[];
  users?: User[];
  connectSessions?: ConnectSession[];
}

interface InjectBody {
  path: string;
  status: number;
  code: string;
  message?: string;
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isArrayOf<T>(value: unknown, guard: (v: unknown) => v is T): value is T[] {
  return Array.isArray(value) && value.every(guard);
}

function isSite(value: unknown): value is Site {
  return isObject(value) && typeof value.id === "string" && typeof value.userId === "string";
}

function isToken(value: unknown): value is Token {
  return isObject(value) && typeof value.full === "string" && typeof value.userId === "string";
}

function isUser(value: unknown): value is User {
  return isObject(value) && typeof value.id === "string" && typeof value.email === "string";
}

function isConnectSession(value: unknown): value is ConnectSession {
  return isObject(value) && typeof value.sessionId === "string";
}

function isInjectBody(value: unknown): value is InjectBody {
  return (
    isObject(value) &&
    typeof value.path === "string" &&
    typeof value.status === "number" &&
    typeof value.code === "string" &&
    (value.message === undefined || typeof value.message === "string")
  );
}

export const controlRouter = new Hono();

controlRouter.post("/reset", (c) => {
  state.reset();
  return c.json(ok({ reset: true }));
});

controlRouter.post("/seed", async (c) => {
  let body: unknown;
  try {
    body = await c.req.json();
  } catch {
    return errorResponse(c, "VALIDATION_ERROR", "seed body must be JSON");
  }
  if (!isObject(body)) {
    return errorResponse(c, "VALIDATION_ERROR", "seed body must be a JSON object");
  }
  const seed = body as SeedBody;

  if (seed.users !== undefined) {
    if (!isArrayOf(seed.users, isUser)) {
      return errorResponse(c, "VALIDATION_ERROR", "users must be User[]");
    }
    for (const user of seed.users) state.users.set(user.id, user);
  }
  if (seed.tokens !== undefined) {
    if (!isArrayOf(seed.tokens, isToken)) {
      return errorResponse(c, "VALIDATION_ERROR", "tokens must be Token[]");
    }
    for (const token of seed.tokens) state.tokens.set(token.full, token);
  }
  if (seed.sites !== undefined) {
    if (!isArrayOf(seed.sites, isSite)) {
      return errorResponse(c, "VALIDATION_ERROR", "sites must be Site[]");
    }
    for (const site of seed.sites) state.sites.set(site.id, site);
  }
  if (seed.connectSessions !== undefined) {
    if (!isArrayOf(seed.connectSessions, isConnectSession)) {
      return errorResponse(c, "VALIDATION_ERROR", "connectSessions must be ConnectSession[]");
    }
    for (const session of seed.connectSessions) {
      state.connectSessions.set(session.sessionId, session);
    }
  }

  return c.json(ok({ seeded: true }));
});

controlRouter.post("/inject", async (c) => {
  let body: unknown;
  try {
    body = await c.req.json();
  } catch {
    return errorResponse(c, "VALIDATION_ERROR", "inject body must be JSON");
  }
  if (!isInjectBody(body)) {
    return errorResponse(
      c,
      "VALIDATION_ERROR",
      "inject body requires path/status/code (message optional)",
    );
  }
  const failure: InjectedFailure = {
    path: body.path,
    status: body.status,
    code: body.code,
    ...(body.message !== undefined ? { message: body.message } : {}),
  };
  state.injectedFailures.set(body.path, failure);
  return c.json(ok({ injected: true, key: body.path }));
});

type DeviceAction = "approve" | "deny" | "slow_down_once" | "expire";

interface DeviceApproveBody {
  user_code: string;
  action: DeviceAction;
}

function isDeviceApproveBody(value: unknown): value is DeviceApproveBody {
  if (!isObject(value)) return false;
  if (typeof value.user_code !== "string") return false;
  if (typeof value.action !== "string") return false;
  return (
    value.action === "approve" ||
    value.action === "deny" ||
    value.action === "slow_down_once" ||
    value.action === "expire"
  );
}

// `POST /__test/device/approve` — drive a device-flow grant through
// the test harness. Body: `{ user_code, action }`. `action`:
//
// - `approve`         — flip grant to `authorized`; next `/token`
//                       poll mints a `tk_*`.
// - `deny`            — flip to `denied`; next poll returns `access_denied`.
// - `slow_down_once`  — next poll returns `slow_down`, then back to pending.
// - `expire`          — flip to `expired`; next poll returns `expired_token`.
controlRouter.post("/device/approve", async (c) => {
  let body: unknown;
  try {
    body = await c.req.json();
  } catch {
    return errorResponse(c, "VALIDATION_ERROR", "device approve body must be JSON");
  }
  if (!isDeviceApproveBody(body)) {
    return errorResponse(
      c,
      "VALIDATION_ERROR",
      "device approve body requires { user_code, action }",
    );
  }
  const deviceCode = state.userCodeIndex.get(body.user_code);
  if (!deviceCode) {
    return errorResponse(c, "NOT_FOUND", `unknown user_code: ${body.user_code}`);
  }
  const grant = state.deviceGrants.get(deviceCode);
  if (!grant) {
    return errorResponse(c, "NOT_FOUND", `unknown device_code: ${deviceCode}`);
  }
  switch (body.action) {
    case "approve":
      grant.status = "authorized";
      break;
    case "deny":
      grant.status = "denied";
      break;
    case "slow_down_once":
      grant.status = "slow_down_once";
      break;
    case "expire":
      grant.status = "expired";
      break;
  }
  return c.json(ok({ deviceCode, status: grant.status }));
});

// `POST /__test/sites/:id/rename` — back-door rename without going
// through the bearer-authenticated PATCH path. Used by the deploy E2E
// to simulate a rename performed on a *different* machine, so the pin
// store on the machine under test stays at the original (now stale)
// name. Body: `{ name }`.
controlRouter.post("/sites/:id/rename", async (c) => {
  const id = c.req.param("id");
  let body: unknown;
  try {
    body = await c.req.json();
  } catch {
    return errorResponse(c, "VALIDATION_ERROR", "rename body must be JSON");
  }
  if (
    typeof body !== "object" ||
    body === null ||
    typeof (body as { name?: unknown }).name !== "string"
  ) {
    return errorResponse(c, "VALIDATION_ERROR", "rename body requires { name }");
  }
  const site = state.sites.get(id);
  if (!site) {
    return errorResponse(c, "NOT_FOUND", `unknown site id: ${id}`);
  }
  site.projectName = (body as { name: string }).name;
  return c.json(ok({ id, projectName: site.projectName }));
});

controlRouter.get("/state", (c) => {
  return c.json(ok(state.snapshot()));
});
