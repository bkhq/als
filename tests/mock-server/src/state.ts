import { seedDefaults } from "./fixtures";

// All wire shapes mirror the live server: camelCase identifiers, ISO-8601
// timestamps, no `state` field on Site (lifecycle is derived from
// `expiresAt`/`releasedAt`). Test-only fixture knobs (`passwordFixture`)
// remain on the in-memory record but are stripped before serialisation.

export interface Version {
  id: string;
  siteId: string;
  /** ISO-8601 timestamp. */
  createdAt: string;
  sizeBytes: number;
  fileCount: number;
  note: string | null;
}

export interface Site {
  id: string;
  userId: string;
  fullDomain: string;
  projectName: string;
  description: string | null;
  authRequired: boolean;
  passwordHash: string | null;
  currentVersion: string;
  /** ISO-8601 timestamp. */
  createdAt: string;
  /** ISO-8601 timestamp. */
  updatedAt: string;
  /** ISO-8601 timestamp, or `null` for "never expires". */
  expiresAt: string | null;
  /** ISO-8601 timestamp once released. */
  releasedAt: string | null;
  versions: Version[];
  sizeBytes: number;
  /**
   * Test-only knob: when present, `POST sites/:id/password` with
   * `password: "auto"` returns this exact string in the response so
   * E2E tests can pin a deterministic `Password: <p>` assertion.
   */
  passwordFixture?: string;
}

export interface Token {
  prefix: string;
  full: string;
  userId: string;
  name: string;
  /** ISO-8601 timestamp. */
  createdAt: string;
}

export type ConnectStatus = "pending" | "authorized" | "expired";

export interface ConnectSession {
  sessionId: string;
  userId: string | null;
  token: string | null;
  tokenPrefix: string | null;
  status: ConnectStatus;
  /** ISO-8601 timestamp. */
  createdAt: string;
  /** ISO-8601 timestamp. */
  expiresAt: string;
}

/**
 * RFC 8628 device-flow grant. Mirrors the live server's
 * `device_grants` row minus secrets that never leave the server. Test
 * harnesses drive the lifecycle via `POST /__test/device/approve`.
 *
 * - `pending`         — keep polling
 * - `slow_down_once`  — server emitted a slow_down; flip back to pending
 * - `denied`          — `POST /token` returns `access_denied`
 * - `authorized`      — `POST /token` mints (and re-returns) `tk_*`
 * - `expired`         — `POST /token` returns `expired_token`
 */
export type DeviceGrantStatus =
  | "pending"
  | "slow_down_once"
  | "denied"
  | "authorized"
  | "expired";

export interface DeviceGrant {
  deviceCode: string;
  userCode: string;
  clientId: string;
  hostnameHint: string | null;
  status: DeviceGrantStatus;
  /** Seconds the server tells the CLI to wait between polls. */
  interval: number;
  /** Token minted on first `authorized` poll (echoed thereafter). */
  token: string | null;
  /** Unix seconds for the device_code's TTL. */
  expiresAtSec: number;
}

export interface Quota {
  maxSites: number;
  maxSizeBytes: number;
  maxExtractedBytes: number;
  maxFilesPerSite: number;
  totalSites: number;
  totalBytes: number;
}

export interface User {
  id: string;
  username: string;
  email: string;
  name: string;
  role: string;
  /** ISO-8601 timestamp. */
  createdAt: string;
  quota: Quota;
}

export interface InjectedFailure {
  path: string;
  status: number;
  code: string;
  message?: string;
}

export interface State {
  sites: Map<string, Site>;
  tokens: Map<string, Token>;
  connectSessions: Map<string, ConnectSession>;
  /** Keyed by `deviceCode`. */
  deviceGrants: Map<string, DeviceGrant>;
  /** Keyed by `userCode` -> `deviceCode`. */
  userCodeIndex: Map<string, string>;
  users: Map<string, User>;
  injectedFailures: Map<string, InjectedFailure>;
  reset(): void;
  snapshot(): unknown;
}

function makeEmpty(): Omit<State, "reset" | "snapshot"> {
  return {
    sites: new Map(),
    tokens: new Map(),
    connectSessions: new Map(),
    deviceGrants: new Map(),
    userCodeIndex: new Map(),
    users: new Map(),
    injectedFailures: new Map(),
  };
}

function clear(s: State): void {
  s.sites.clear();
  s.tokens.clear();
  s.connectSessions.clear();
  s.deviceGrants.clear();
  s.userCodeIndex.clear();
  s.users.clear();
  s.injectedFailures.clear();
}

export const state: State = {
  ...makeEmpty(),
  reset(): void {
    clear(this);
    seedDefaults(this);
  },
  snapshot(): unknown {
    return {
      sites: Array.from(this.sites.values()),
      tokens: Array.from(this.tokens.values()),
      connectSessions: Array.from(this.connectSessions.values()),
      deviceGrants: Array.from(this.deviceGrants.values()),
      users: Array.from(this.users.values()),
      injectedFailures: Array.from(this.injectedFailures.entries()).map(
        ([key, value]) => ({ key, ...value }),
      ),
    };
  },
};

seedDefaults(state);
