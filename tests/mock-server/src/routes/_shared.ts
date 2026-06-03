import type { Context } from "hono";

import { errorResponse, type ErrorCode, type HttpStatus } from "../envelope";
import { state, type InjectedFailure } from "../state";

// Match the CLI's `looks_like_id` regex (`[a-z2-7]{10}` = standard
// base32 lowercase). Including 0/1/8/9 here used to produce ids the
// CLI then rejected as ids and tried to resolve as names.
const ID_ALPHABET = "abcdefghijklmnopqrstuvwxyz234567";
const PASSWORD_ALPHABET = "abcdefghjkmnpqrstuvwxyz23456789";
const HEX_ALPHABET = "0123456789abcdef";

export const DEFAULT_DOMAIN_ROOT = "a.ls";
const DEFAULT_DURATION_SECONDS = 168 * 3600;

export function randomString(length: number, alphabet: string): string {
  const buf = new Uint8Array(length);
  crypto.getRandomValues(buf);
  let out = "";
  for (let i = 0; i < length; i++) {
    out += alphabet[buf[i]! % alphabet.length];
  }
  return out;
}

export function generateSiteId(): string {
  return randomString(10, ID_ALPHABET);
}

export function generatePassword(): string {
  const segs = [
    randomString(3, PASSWORD_ALPHABET),
    randomString(3, PASSWORD_ALPHABET),
    randomString(3, PASSWORD_ALPHABET),
  ];
  return segs.join("-");
}

export function generateVersion(now: Date): string {
  const pad = (n: number, w = 2): string => n.toString().padStart(w, "0");
  const yyyy = now.getUTCFullYear();
  const mm = pad(now.getUTCMonth() + 1);
  const dd = pad(now.getUTCDate());
  const hh = pad(now.getUTCHours());
  const mi = pad(now.getUTCMinutes());
  const ss = pad(now.getUTCSeconds());
  const suffix = randomString(6, HEX_ALPHABET);
  return `${yyyy}${mm}${dd}-${hh}${mi}${ss}-${suffix}`;
}

export function generateTokenFull(): string {
  return `tk_${randomString(16, HEX_ALPHABET)}`;
}

export function nowSeconds(): number {
  return Math.floor(Date.now() / 1000);
}

export function nowIso(): string {
  return new Date().toISOString();
}

export function isoFromSeconds(sec: number): string {
  return new Date(sec * 1000).toISOString();
}

interface DurationOk {
  ok: true;
  expiresAtIso: string | null;
}
interface DurationErr {
  ok: false;
  message: string;
}
export type DurationResult = DurationOk | DurationErr;

const UNIT_SECONDS: Record<string, number> = {
  s: 1,
  m: 60,
  h: 3600,
  d: 86_400,
  w: 604_800,
  mo: 2_592_000,
  y: 31_536_000,
};

export function parseDuration(input: string, nowSec: number): DurationResult {
  const trimmed = input.trim();
  if (trimmed.length === 0) {
    return { ok: false, message: "duration cannot be empty" };
  }
  if (trimmed === "never") {
    return { ok: true, expiresAtIso: null };
  }
  const iso = Date.parse(trimmed);
  if (!Number.isNaN(iso)) {
    return { ok: true, expiresAtIso: new Date(iso).toISOString() };
  }
  const match = /^(\d+)(mo|[smhdwy])$/.exec(trimmed);
  if (!match) {
    return { ok: false, message: `unrecognized duration: ${input}` };
  }
  const value = Number.parseInt(match[1]!, 10);
  const unit = match[2]!;
  const seconds = UNIT_SECONDS[unit];
  if (seconds === undefined) {
    return { ok: false, message: `unrecognized duration unit: ${unit}` };
  }
  return { ok: true, expiresAtIso: isoFromSeconds(nowSec + value * seconds) };
}

export function defaultExpiresIso(nowSec: number): string {
  return isoFromSeconds(nowSec + DEFAULT_DURATION_SECONDS);
}

function statusToCode(failure: InjectedFailure): ErrorCode {
  return failure.code as ErrorCode;
}

// Canonical Retry-After value emitted with every injected `RATE_LIMITED`
// failure. The CLI E2E matrix pins this value so tests can assert that
// the header round-trips into stderr.
const RATE_LIMITED_RETRY_AFTER_SECONDS = "30";

export function consumeInjection(c: Context): Response | null {
  const key = `${c.req.method} ${c.req.path}`;
  const failure = state.injectedFailures.get(key);
  if (!failure) return null;
  state.injectedFailures.delete(key);
  const code = statusToCode(failure);
  const message = failure.message ?? code;
  if (code === "RATE_LIMITED") {
    c.header("Retry-After", RATE_LIMITED_RETRY_AFTER_SECONDS);
  }
  return c.json(
    { success: false as const, error: { code, message } },
    failure.status as HttpStatus,
  );
}

export function notFound(c: Context, what: string): Response {
  return errorResponse(c, "NOT_FOUND", `${what} not found`);
}

export function fullDomainFor(id: string): string {
  return `${id}.${DEFAULT_DOMAIN_ROOT}`;
}

export function urlFor(id: string): string {
  return `https://${fullDomainFor(id)}`;
}
