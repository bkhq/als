import { Hono } from "hono";

import { requireBearer } from "../auth";
import { errorResponse, ok } from "../envelope";
import { state, type ConnectSession, type Token } from "../state";
import {
  consumeInjection,
  generateTokenFull,
  isoFromSeconds,
  nowSeconds,
  notFound,
} from "./_shared";

export const connectRouter = new Hono();

interface StartBody {
  hostname_hint?: string;
}

function isStartBody(value: unknown): value is StartBody {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  const v = value as Record<string, unknown>;
  if (v.hostname_hint !== undefined && typeof v.hostname_hint !== "string") return false;
  return true;
}

interface AuthorizeBody {
  token_name?: string;
}

function isAuthorizeBody(value: unknown): value is AuthorizeBody {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  const v = value as Record<string, unknown>;
  if (v.token_name !== undefined && typeof v.token_name !== "string") return false;
  return true;
}

const PAIRING_TTL_SECONDS = 300;

function generateSessionId(): string {
  const buf = new Uint8Array(8);
  crypto.getRandomValues(buf);
  return Array.from(buf, (b) => b.toString(16).padStart(2, "0")).join("");
}

// `POST /api/cli/connect/start` — mint a pending session. Unauthenticated.
connectRouter.post("/start", async (c) => {
  const injected = consumeInjection(c);
  if (injected) return injected;

  let body: unknown = {};
  const raw = await c.req.text();
  if (raw.length > 0) {
    try {
      body = JSON.parse(raw);
    } catch {
      return errorResponse(c, "VALIDATION_ERROR", "body must be JSON");
    }
  }
  if (!isStartBody(body)) {
    return errorResponse(c, "VALIDATION_ERROR", "unexpected fields or wrong types");
  }

  const sessionId = generateSessionId();
  const now = nowSeconds();
  const session: ConnectSession = {
    sessionId,
    userId: null,
    token: null,
    tokenPrefix: null,
    status: "pending",
    createdAt: isoFromSeconds(now),
    expiresAt: isoFromSeconds(now + PAIRING_TTL_SECONDS),
  };
  state.connectSessions.set(sessionId, session);

  return c.json(
    ok({
      sessionId,
      authorizeUrl: `https://a.ls/cli/connect?session=${sessionId}`,
      expiresAt: session.expiresAt,
    }),
    201,
  );
});

// `GET /api/cli/connect/:sid/status` — CLI-side poll. Unauthenticated.
connectRouter.get("/:sid/status", (c) => {
  const injected = consumeInjection(c);
  if (injected) return injected;

  const sid = c.req.param("sid");
  if (!sid) return notFound(c, "connect session");

  const session = state.connectSessions.get(sid);
  if (!session) return notFound(c, "connect session");

  if (session.status === "expired") {
    return errorResponse(c, "PAIRING_EXPIRED", "connect session expired");
  }
  if (session.status === "pending") {
    return c.json(ok({ status: "pending" as const }));
  }
  return c.json(
    ok({
      status: "authorized" as const,
      token: session.token,
      tokenPrefix: session.tokenPrefix,
    }),
  );
});

// `POST /api/cli/connect/:sid/authorize` — browser side. Test harness
// drives this directly with a bearer to simulate the console.
connectRouter.post("/:sid/authorize", requireBearer, async (c) => {
  const injected = consumeInjection(c);
  if (injected) return injected;

  const sid = c.req.param("sid");
  if (!sid) return errorResponse(c, "VALIDATION_ERROR", "session id required");

  const session = state.connectSessions.get(sid);
  if (!session) return notFound(c, "connect session");
  if (session.status === "expired") {
    return errorResponse(c, "PAIRING_EXPIRED", "connect session expired");
  }
  if (session.status === "authorized") {
    return errorResponse(c, "PAIRING_ALREADY_AUTHORIZED", "already authorized");
  }

  let body: unknown = {};
  const raw = await c.req.text();
  if (raw.length > 0) {
    try {
      body = JSON.parse(raw);
    } catch {
      return errorResponse(c, "VALIDATION_ERROR", "body must be JSON");
    }
  }
  if (!isAuthorizeBody(body)) {
    return errorResponse(c, "VALIDATION_ERROR", "unexpected fields or wrong types");
  }

  const user = c.get("user");
  const now = nowSeconds();
  const tokenFull = generateTokenFull();
  const tokenName = body.token_name ?? "cli-pairing";
  const newToken: Token = {
    prefix: tokenFull.slice(0, 8),
    full: tokenFull,
    userId: user.id,
    name: tokenName,
    createdAt: isoFromSeconds(now),
  };
  state.tokens.set(newToken.full, newToken);

  session.userId = user.id;
  session.token = tokenFull;
  session.tokenPrefix = newToken.prefix;
  session.status = "authorized";

  return c.json(
    ok({
      sessionId: session.sessionId,
      tokenName,
      tokenPrefix: newToken.prefix,
    }),
  );
});
