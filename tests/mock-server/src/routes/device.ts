import { Hono } from "hono";

import { state, type DeviceGrant, type Token } from "../state";
import {
  consumeInjection,
  generateTokenFull,
  isoFromSeconds,
  nowSeconds,
} from "./_shared";

export const deviceRouter = new Hono();

// 15 minutes — matches the live server.
const DEVICE_TTL_SECONDS = 900;
const DEFAULT_POLL_INTERVAL_SECONDS = 5;
const VERIFICATION_URL_ROOT = "https://a.ls/verify";
const GRANT_TYPE = "urn:ietf:params:oauth:grant-type:device_code";

function generateDeviceCode(): string {
  const buf = new Uint8Array(16);
  crypto.getRandomValues(buf);
  return Array.from(buf, (b) => b.toString(16).padStart(2, "0")).join("");
}

function generateUserCode(): string {
  const alphabet = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
  const pick = (n: number): string => {
    const buf = new Uint8Array(n);
    crypto.getRandomValues(buf);
    let out = "";
    for (let i = 0; i < n; i++) {
      out += alphabet[buf[i]! % alphabet.length];
    }
    return out;
  };
  return `${pick(4)}-${pick(4)}`;
}

async function readForm(c: import("hono").Context): Promise<Record<string, string>> {
  const ct = c.req.header("content-type") ?? "";
  if (ct.includes("application/x-www-form-urlencoded")) {
    const text = await c.req.text();
    return Object.fromEntries(new URLSearchParams(text));
  }
  if (ct.includes("application/json")) {
    const raw = await c.req.text();
    if (!raw) return {};
    try {
      const obj = JSON.parse(raw);
      if (obj && typeof obj === "object" && !Array.isArray(obj)) {
        const out: Record<string, string> = {};
        for (const [k, v] of Object.entries(obj as Record<string, unknown>)) {
          if (typeof v === "string") out[k] = v;
        }
        return out;
      }
    } catch {
      /* fall through to empty */
    }
  }
  return {};
}

// `POST /api/auth/device/authorize` — start a device-flow grant.
// Unauthenticated. RFC 8628 §3.2 plain-shape response.
deviceRouter.post("/authorize", async (c) => {
  const injected = consumeInjection(c);
  if (injected) return injected;

  const form = await readForm(c);
  if (form.client_id !== "toss-cli") {
    return c.json({ error: "invalid_client" }, 401);
  }

  const deviceCode = generateDeviceCode();
  const userCode = generateUserCode();
  const now = nowSeconds();
  const grant: DeviceGrant = {
    deviceCode,
    userCode,
    clientId: form.client_id,
    hostnameHint: form.hostname_hint ?? null,
    status: "pending",
    interval: DEFAULT_POLL_INTERVAL_SECONDS,
    token: null,
    expiresAtSec: now + DEVICE_TTL_SECONDS,
  };
  state.deviceGrants.set(deviceCode, grant);
  state.userCodeIndex.set(userCode, deviceCode);

  return c.json({
    device_code: deviceCode,
    user_code: userCode,
    verification_uri: VERIFICATION_URL_ROOT,
    verification_uri_complete: `${VERIFICATION_URL_ROOT}?user_code=${userCode}`,
    expires_in: DEVICE_TTL_SECONDS,
    interval: grant.interval,
  });
});

// `POST /api/auth/device/token` — single poll. Unauthenticated.
// RFC 8628 §3.5 plain-shape error responses, plain-shape success body.
deviceRouter.post("/token", async (c) => {
  const injected = consumeInjection(c);
  if (injected) return injected;

  const form = await readForm(c);
  if (form.client_id !== "toss-cli") {
    return c.json({ error: "invalid_client" }, 401);
  }
  if (form.grant_type !== GRANT_TYPE) {
    return c.json({ error: "unsupported_grant_type" }, 400);
  }
  const deviceCode = form.device_code ?? "";
  if (!deviceCode) {
    return c.json({ error: "expired_token" }, 400);
  }

  const grant = state.deviceGrants.get(deviceCode);
  if (!grant) {
    return c.json({ error: "expired_token" }, 400);
  }
  if (nowSeconds() >= grant.expiresAtSec || grant.status === "expired") {
    grant.status = "expired";
    return c.json({ error: "expired_token" }, 400);
  }

  switch (grant.status) {
    case "pending":
      return c.json({ error: "authorization_pending" }, 400);
    case "slow_down_once":
      grant.status = "pending";
      return c.json({ error: "slow_down" }, 400);
    case "denied":
      return c.json({ error: "access_denied" }, 400);
    case "authorized": {
      if (grant.token === null) {
        const fullToken = generateTokenFull();
        const tokenRecord: Token = {
          prefix: fullToken.slice(0, 8),
          full: fullToken,
          userId: "u_alice",
          name: `cli-${grant.hostnameHint ?? "device"}`,
          createdAt: isoFromSeconds(nowSeconds()),
        };
        state.tokens.set(fullToken, tokenRecord);
        grant.token = fullToken;
      }
      return c.json({
        access_token: grant.token,
        token_type: "Bearer",
        expires_in: 7200,
      });
    }
    default:
      return c.json({ error: "authorization_pending" }, 400);
  }
});
