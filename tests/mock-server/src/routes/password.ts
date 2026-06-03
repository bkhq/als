import { type Context, Hono } from "hono";

import { requireBearer } from "../auth";
import { errorResponse, ok } from "../envelope";
import { state, type Site } from "../state";
import { consumeInjection, generatePassword, nowIso, notFound } from "./_shared";

export const passwordRouter = new Hono();

passwordRouter.use("*", requireBearer);

function findOwnedSite(c: Context): Site | "missing" | "forbidden" {
  const id = c.req.param("id");
  if (!id) return "missing";
  const site = state.sites.get(id);
  if (!site || site.releasedAt !== null) return "missing";
  if (site.userId !== c.get("user").id) return "forbidden";
  return site;
}

interface PasswordBody {
  password?: string | null;
}

function isPasswordBody(value: unknown): value is PasswordBody {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  const v = value as Record<string, unknown>;
  if (v.password !== undefined && v.password !== null && typeof v.password !== "string")
    return false;
  return true;
}

function serialiseSite(site: Site): Record<string, unknown> {
  const { passwordFixture: _f, passwordHash: _h, versions: _v, ...wire } = site;
  return wire;
}

// `POST /api/sites/:id/password`: collapse "set / replace / clear" into
// one route. `password: "auto"` asks the mock to generate a memorable
// string and echo it via the response's `generatedPassword` field so
// tests can assert on it. `password: null` clears the password (the live
// server's audit op is `"cleared"`).
passwordRouter.post("/:id/password", async (c) => {
  const injected = consumeInjection(c);
  if (injected) return injected;

  const found = findOwnedSite(c);
  if (found === "missing") return notFound(c, "site");
  if (found === "forbidden") return errorResponse(c, "FORBIDDEN", "site not accessible");

  let body: unknown = {};
  const raw = await c.req.text();
  if (raw.length > 0) {
    try {
      body = JSON.parse(raw);
    } catch {
      return errorResponse(c, "VALIDATION_ERROR", "body must be JSON");
    }
  }
  if (!isPasswordBody(body)) {
    return errorResponse(c, "VALIDATION_ERROR", "unexpected fields or wrong types");
  }

  let op: "set" | "cleared";
  let generated: string | undefined;
  if (body.password === null) {
    if (found.passwordHash === null) {
      return errorResponse(c, "VALIDATION_ERROR", "site has no password to clear");
    }
    found.passwordHash = null;
    found.authRequired = false;
    op = "cleared";
  } else {
    let plain = body.password ?? "";
    if (plain.length === 0) {
      return errorResponse(c, "VALIDATION_ERROR", "password must be a non-empty string or null");
    }
    if (plain === "auto") {
      generated = found.passwordFixture ?? generatePassword();
      plain = generated;
    }
    found.passwordHash = plain;
    found.authRequired = true;
    op = "set";
  }
  found.updatedAt = nowIso();

  return c.json(
    ok({
      ...serialiseSite(found),
      op,
      ...(generated !== undefined ? { generatedPassword: generated } : {}),
    }),
  );
});
