import { type Context, Hono } from "hono";

import { requireBearer } from "../auth";
import { errorResponse, ok, okList } from "../envelope";
import { state, type Site } from "../state";
import { consumeInjection, nowIso, notFound } from "./_shared";

export const sitesRouter = new Hono();

sitesRouter.use("*", requireBearer);

const DEFAULT_LIMIT = 50;
const MAX_LIMIT = 200;

function serialiseSite(site: Site): Record<string, unknown> {
  const { passwordFixture: _f, passwordHash: _h, versions: _v, ...wire } = site;
  return wire;
}

function parsePositiveInt(raw: string | undefined, fallback: number): number | null {
  if (raw === undefined) return fallback;
  if (!/^\d+$/.test(raw)) return null;
  const parsed = Number.parseInt(raw, 10);
  if (!Number.isFinite(parsed) || parsed < 0) return null;
  return parsed;
}

function findOwnedSite(c: Context): Site | "missing" | "forbidden" {
  const id = c.req.param("id");
  if (!id) return "missing";
  const site = state.sites.get(id);
  if (!site || site.releasedAt !== null) return "missing";
  if (site.userId !== c.get("user").id) return "forbidden";
  return site;
}

sitesRouter.get("/", (c) => {
  const injected = consumeInjection(c);
  if (injected) return injected;

  const user = c.get("user");

  const rawLimit = parsePositiveInt(c.req.query("limit"), DEFAULT_LIMIT);
  if (rawLimit === null) {
    return errorResponse(c, "VALIDATION_ERROR", "limit must be a non-negative integer");
  }
  const limit = Math.min(Math.max(rawLimit, 1), MAX_LIMIT);

  const rawPage = parsePositiveInt(c.req.query("page"), 1);
  if (rawPage === null) {
    return errorResponse(c, "VALIDATION_ERROR", "page must be a non-negative integer");
  }
  const page = Math.max(rawPage, 1);

  const showAll = c.req.query("all") === "true" && user.role === "admin";
  const owned = Array.from(state.sites.values()).filter(
    (s) => (showAll || s.userId === user.id) && s.releasedAt === null,
  );
  owned.sort((a, b) => (a.createdAt < b.createdAt ? 1 : -1));
  const offset = (page - 1) * limit;
  const pageRows = owned.slice(offset, offset + limit).map(serialiseSite);

  return c.json(okList(pageRows, { total: owned.length, page, limit }));
});

sitesRouter.get("/:id", (c) => {
  const injected = consumeInjection(c);
  if (injected) return injected;

  const found = findOwnedSite(c);
  if (found === "missing") return notFound(c, "site");
  if (found === "forbidden") return errorResponse(c, "FORBIDDEN", "site not accessible");
  return c.json(ok(serialiseSite(found)));
});

interface PatchBody {
  projectName?: string;
  description?: string | null;
  expiresAt?: string | null;
}

function isPatchBody(value: unknown): value is PatchBody {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  const v = value as Record<string, unknown>;
  if (v.projectName !== undefined && typeof v.projectName !== "string") return false;
  if (
    v.description !== undefined &&
    v.description !== null &&
    typeof v.description !== "string"
  )
    return false;
  if (v.expiresAt !== undefined && v.expiresAt !== null && typeof v.expiresAt !== "string")
    return false;
  return true;
}

sitesRouter.patch("/:id", async (c) => {
  const injected = consumeInjection(c);
  if (injected) return injected;

  const found = findOwnedSite(c);
  if (found === "missing") return notFound(c, "site");
  if (found === "forbidden") return errorResponse(c, "FORBIDDEN", "site not accessible");

  let body: unknown;
  try {
    body = await c.req.json();
  } catch {
    return errorResponse(c, "VALIDATION_ERROR", "body must be JSON");
  }
  if (!isPatchBody(body)) {
    return errorResponse(c, "VALIDATION_ERROR", "unexpected fields or wrong types in patch body");
  }

  let touched = false;
  if (body.projectName !== undefined) {
    found.projectName = body.projectName;
    touched = true;
  }
  if (body.description !== undefined) {
    found.description = body.description;
    touched = true;
  }
  if (body.expiresAt !== undefined) {
    found.expiresAt = body.expiresAt;
    touched = true;
  }
  if (!touched) {
    return errorResponse(c, "VALIDATION_ERROR", "at least one field must be provided");
  }
  found.updatedAt = nowIso();

  return c.json(ok(serialiseSite(found)));
});

sitesRouter.delete("/:id", (c) => {
  const injected = consumeInjection(c);
  if (injected) return injected;

  const found = findOwnedSite(c);
  if (found === "missing") return notFound(c, "site");
  if (found === "forbidden") return errorResponse(c, "FORBIDDEN", "site not accessible");

  found.releasedAt = nowIso();
  return c.json(ok(null));
});
