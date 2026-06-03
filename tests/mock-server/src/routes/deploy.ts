import { Hono } from "hono";

import { requireBearer } from "../auth";
import { errorResponse, ok } from "../envelope";
import { state, type Site, type Version } from "../state";
import {
  consumeInjection,
  defaultExpiresIso,
  fullDomainFor,
  generateSiteId,
  generateVersion,
  isoFromSeconds,
  nowSeconds,
  parseDuration,
  urlFor,
} from "./_shared";

export const deployRouter = new Hono();

deployRouter.use("*", requireBearer);

function pickString(value: unknown): string | undefined {
  if (typeof value === "string") return value;
  if (Array.isArray(value)) {
    const first = value[0];
    if (typeof first === "string") return first;
  }
  return undefined;
}

function pickFile(value: unknown): File | undefined {
  if (value instanceof File) return value;
  if (Array.isArray(value)) {
    const first = value[0];
    if (first instanceof File) return first;
  }
  return undefined;
}

function allocateSiteId(): string {
  for (let i = 0; i < 16; i++) {
    const candidate = generateSiteId();
    if (!state.sites.has(candidate)) return candidate;
  }
  return `${generateSiteId()}${generateSiteId().slice(0, 1)}`;
}

deployRouter.post("/", async (c) => {
  const injected = consumeInjection(c);
  if (injected) return injected;

  let form: Record<string, string | File | (string | File)[]>;
  try {
    form = await c.req.parseBody({ all: true });
  } catch {
    return errorResponse(c, "INVALID_MULTIPART", "failed to parse multipart body");
  }

  const archive = pickFile(form.archive);
  if (!archive) {
    return errorResponse(c, "VALIDATION_ERROR", "archive field is required and must be a file");
  }

  const siteIdHint = pickString(form.site_id);
  const projectName = pickString(form.project_name);
  const expiresInput = pickString(form.expires);
  const password = pickString(form.auth_password);

  const now = nowSeconds();
  const nowIsoStr = isoFromSeconds(now);
  const user = c.get("user");
  const sizeBytes = archive.size;
  const versionId = generateVersion(new Date(now * 1000));

  let parsedExpires: string | null | undefined;
  if (expiresInput !== undefined) {
    const parsed = parseDuration(expiresInput, now);
    if (!parsed.ok) {
      return errorResponse(c, "VALIDATION_ERROR", parsed.message);
    }
    parsedExpires = parsed.expiresAtIso;
  }

  // Identity resolution. `site_id` wins when present — it anchors the
  // request to a specific site even if the local pin's `project_name`
  // is stale (e.g. the user renamed via another machine). When the id
  // is missing, fall back to per-user case-insensitive name dedup.
  let existing: Site | undefined;
  if (siteIdHint) {
    const candidate = state.sites.get(siteIdHint);
    // A released (rm'd) site is a tombstone — the id stays reserved
    // but the site no longer accepts uploads. Treat it the same as
    // unknown id so a stale `als.toml.id` surfaces a clean error
    // instead of silently resurrecting a deleted deployment.
    if (
      !candidate ||
      candidate.userId !== user.id ||
      candidate.releasedAt !== null
    ) {
      return errorResponse(
        c,
        "NOT_FOUND",
        `no site with id '${siteIdHint}' owned by the caller`,
      );
    }
    existing = candidate;
  } else if (projectName) {
    const needle = projectName.toLowerCase();
    for (const candidate of state.sites.values()) {
      if (
        candidate.userId === user.id &&
        candidate.releasedAt === null &&
        candidate.projectName.toLowerCase() === needle
      ) {
        existing = candidate;
        break;
      }
    }
  }

  if (existing) {
    // Id-anchored deploys may also carry a `project_name` — that is the
    // CLI's `--name` flag asking for a rename in the same request.
    // Skip the work when it matches the current name; reject when it
    // collides with another site owned by the same user.
    if (siteIdHint && projectName && projectName !== existing.projectName) {
      const needle = projectName.toLowerCase();
      for (const candidate of state.sites.values()) {
        if (
          candidate.id !== existing.id &&
          candidate.userId === user.id &&
          candidate.projectName.toLowerCase() === needle
        ) {
          return errorResponse(
            c,
            "VALIDATION_ERROR",
            `another site already uses '${projectName}'`,
          );
        }
      }
      existing.projectName = projectName;
    }

    const newVersion: Version = {
      id: versionId,
      siteId: existing.id,
      createdAt: nowIsoStr,
      sizeBytes,
      fileCount: 1,
      note: null,
    };
    existing.versions.push(newVersion);
    existing.currentVersion = versionId;
    existing.updatedAt = nowIsoStr;
    existing.sizeBytes = sizeBytes;
    if (parsedExpires !== undefined) {
      existing.expiresAt = parsedExpires;
    }
    if (password !== undefined) {
      existing.authRequired = password.length > 0;
      existing.passwordHash = password.length > 0 ? password : null;
    }

    return c.json(
      ok({
        id: existing.id,
        url: urlFor(existing.id),
        projectName: existing.projectName,
        version: versionId,
        expiresAt: existing.expiresAt,
        authRequired: existing.authRequired,
      }),
      200,
    );
  }

  const id = allocateSiteId();
  const expiresAt = parsedExpires ?? defaultExpiresIso(now);

  const firstVersion: Version = {
    id: versionId,
    siteId: id,
    createdAt: nowIsoStr,
    sizeBytes,
    fileCount: 1,
    note: null,
  };

  const site: Site = {
    id,
    userId: user.id,
    fullDomain: fullDomainFor(id),
    // The CLI always sends a non-empty `project_name`; this fallback
    // is purely defensive for malformed test payloads.
    projectName: projectName ?? `als-default-${id.slice(0, 4)}`,
    description: null,
    authRequired: password !== undefined && password.length > 0,
    passwordHash: password !== undefined && password.length > 0 ? password : null,
    currentVersion: versionId,
    createdAt: nowIsoStr,
    updatedAt: nowIsoStr,
    expiresAt,
    releasedAt: null,
    versions: [firstVersion],
    sizeBytes,
  };

  state.sites.set(id, site);

  return c.json(
    ok({
      id,
      url: urlFor(id),
      projectName: site.projectName,
      version: versionId,
      expiresAt: site.expiresAt,
      authRequired: site.authRequired,
    }),
    201,
  );
});
