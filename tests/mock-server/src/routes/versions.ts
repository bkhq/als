import { type Context, Hono } from "hono";

import { requireBearer } from "../auth";
import { errorResponse, ok } from "../envelope";
import { state, type Site } from "../state";
import { consumeInjection, nowIso, notFound } from "./_shared";

export const versionsRouter = new Hono();

versionsRouter.use("*", requireBearer);

function findOwnedSite(c: Context): Site | "missing" | "forbidden" {
  const id = c.req.param("id");
  if (!id) return "missing";
  const site = state.sites.get(id);
  if (!site || site.releasedAt !== null) return "missing";
  if (site.userId !== c.get("user").id) return "forbidden";
  return site;
}

versionsRouter.get("/:id/versions", (c) => {
  const injected = consumeInjection(c);
  if (injected) return injected;

  const found = findOwnedSite(c);
  if (found === "missing") return notFound(c, "site");
  if (found === "forbidden") return errorResponse(c, "FORBIDDEN", "site not accessible");

  const items = [...found.versions].sort((a, b) => (a.createdAt < b.createdAt ? 1 : -1));
  return c.json(ok(items));
});

interface ActivateBody {
  version?: string;
}

function isActivateBody(value: unknown): value is ActivateBody {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  const v = value as Record<string, unknown>;
  if (v.version !== undefined && typeof v.version !== "string") return false;
  return true;
}

// `POST /api/sites/:id/activate`: the live server does not implement
// this endpoint; the mock continues to provide it so CLI E2E coverage
// for `als site <id> --version <v>` (rollback) keeps working.
versionsRouter.post("/:id/activate", async (c) => {
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
  if (!isActivateBody(body) || body.version === undefined) {
    return errorResponse(c, "VALIDATION_ERROR", "version field is required");
  }

  const exists = found.versions.find((v) => v.id === body.version);
  if (!exists) return notFound(c, "version");

  found.currentVersion = exists.id;
  found.updatedAt = nowIso();
  return c.json(
    ok({
      id: found.id,
      currentVersion: found.currentVersion,
      activatedVersion: exists,
    }),
  );
});
