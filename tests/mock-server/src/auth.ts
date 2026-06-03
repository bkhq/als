import type { MiddlewareHandler } from "hono";

import { errorResponse } from "./envelope";
import { state, type Token, type User } from "./state";

declare module "hono" {
  interface ContextVariableMap {
    user: User;
    token: Token;
  }
}

function lookupToken(bearer: string): { user: User; token: Token } | null {
  const token = state.tokens.get(bearer);
  if (!token) return null;
  const user = state.users.get(token.userId);
  if (!user) return null;
  return { user, token };
}

export const requireBearer: MiddlewareHandler = async (c, next) => {
  const header = c.req.header("Authorization") ?? "";
  if (!header.startsWith("Bearer ")) {
    return errorResponse(c, "UNAUTHORIZED", "missing bearer token");
  }
  const bearer = header.slice("Bearer ".length).trim();
  if (!bearer) {
    return errorResponse(c, "UNAUTHORIZED", "empty bearer token");
  }
  const found = lookupToken(bearer);
  if (!found) {
    return errorResponse(c, "UNAUTHORIZED", "unknown bearer token");
  }
  c.set("user", found.user);
  c.set("token", found.token);
  await next();
};
