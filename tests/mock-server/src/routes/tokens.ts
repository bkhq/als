import { Hono } from "hono";

import { requireBearer } from "../auth";
import { errorResponse, ok } from "../envelope";
import { state } from "../state";
import { consumeInjection, notFound } from "./_shared";

export const tokensRouter = new Hono();

tokensRouter.use("*", requireBearer);

tokensRouter.get("/", (c) => {
  const injected = consumeInjection(c);
  if (injected) return injected;

  const user = c.get("user");
  const items = Array.from(state.tokens.values())
    .filter((t) => t.userId === user.id)
    .map((t) => ({
      prefix: t.prefix,
      name: t.name,
      createdAt: t.createdAt,
    }));
  return c.json(ok(items));
});

tokensRouter.delete("/:prefix", (c) => {
  const injected = consumeInjection(c);
  if (injected) return injected;

  const prefix = c.req.param("prefix");
  if (!prefix) return notFound(c, "token");

  const user = c.get("user");
  let removedFull: string | null = null;
  for (const [full, token] of state.tokens.entries()) {
    if (token.prefix === prefix) {
      if (token.userId !== user.id) {
        return errorResponse(c, "FORBIDDEN", "token not accessible");
      }
      removedFull = full;
      break;
    }
  }
  if (removedFull === null) return notFound(c, "token");

  state.tokens.delete(removedFull);
  return c.json(ok(null));
});
