import { Hono } from "hono";

import { requireBearer } from "../auth";
import { ok } from "../envelope";
import { consumeInjection } from "./_shared";

export const accountRouter = new Hono();

accountRouter.use("*", requireBearer);

accountRouter.get("/me", (c) => {
  const injected = consumeInjection(c);
  if (injected) return injected;

  const user = c.get("user");
  return c.json(
    ok({
      id: user.id,
      username: user.username,
      name: user.name,
      email: user.email,
      role: user.role,
      avatar: null,
      status: "active",
      lastLoginAt: null,
      createdAt: user.createdAt,
      groups: [],
    }),
  );
});
