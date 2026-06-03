import { Hono } from "hono";

import { requireBearer } from "../auth";
import { ok } from "../envelope";
import { state } from "../state";
import { consumeInjection } from "./_shared";

export const quotaRouter = new Hono();

quotaRouter.use("*", requireBearer);

quotaRouter.get("/me", (c) => {
  const injected = consumeInjection(c);
  if (injected) return injected;

  const user = c.get("user");
  let totalSites = 0;
  let totalBytes = 0;
  for (const site of state.sites.values()) {
    if (site.userId !== user.id) continue;
    if (site.releasedAt !== null) continue;
    totalSites += 1;
    totalBytes += site.sizeBytes;
  }
  return c.json(
    ok({
      userId: user.id,
      maxSites: user.quota.maxSites,
      maxSizeBytes: user.quota.maxSizeBytes,
      maxExtractedBytes: user.quota.maxExtractedBytes,
      maxFilesPerSite: user.quota.maxFilesPerSite,
      totalSites,
      totalBytes,
      notifyEmail: null,
      notifyOnExpire: true,
      notifyOnQuota: true,
    }),
  );
});
