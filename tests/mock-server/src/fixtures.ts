import type { State, Token, User } from "./state";

export const DEFAULT_USER_ID = "u_alice";
export const DEFAULT_USER_USERNAME = "alice";
export const DEFAULT_USER_EMAIL = "alice@example.com";
export const DEFAULT_USER_NAME = "Alice Liu";
export const DEFAULT_USER_ROLE = "user";

export const DEFAULT_TOKEN_FULL = "tk_test0001abcdef";
export const DEFAULT_TOKEN_PREFIX = DEFAULT_TOKEN_FULL.slice(0, 8);
export const DEFAULT_TOKEN_NAME = "default-test-token";

export const DEFAULT_QUOTA_MAX_SITES = 100;
export const DEFAULT_QUOTA_MAX_SIZE_BYTES = 52_428_800;
export const DEFAULT_QUOTA_MAX_EXTRACTED_BYTES = 209_715_200;
export const DEFAULT_QUOTA_MAX_FILES_PER_SITE = 1_000;

const DEFAULT_CREATED_AT_ISO = "2026-05-01T00:00:00.000Z";

export function defaultUser(): User {
  return {
    id: DEFAULT_USER_ID,
    username: DEFAULT_USER_USERNAME,
    email: DEFAULT_USER_EMAIL,
    name: DEFAULT_USER_NAME,
    role: DEFAULT_USER_ROLE,
    createdAt: DEFAULT_CREATED_AT_ISO,
    quota: {
      maxSites: DEFAULT_QUOTA_MAX_SITES,
      maxSizeBytes: DEFAULT_QUOTA_MAX_SIZE_BYTES,
      maxExtractedBytes: DEFAULT_QUOTA_MAX_EXTRACTED_BYTES,
      maxFilesPerSite: DEFAULT_QUOTA_MAX_FILES_PER_SITE,
      totalSites: 0,
      totalBytes: 0,
    },
  };
}

export function defaultToken(): Token {
  return {
    prefix: DEFAULT_TOKEN_PREFIX,
    full: DEFAULT_TOKEN_FULL,
    userId: DEFAULT_USER_ID,
    name: DEFAULT_TOKEN_NAME,
    createdAt: DEFAULT_CREATED_AT_ISO,
  };
}

export function seedDefaults(s: State): void {
  const user = defaultUser();
  s.users.set(user.id, user);
  const token = defaultToken();
  s.tokens.set(token.full, token);
}
