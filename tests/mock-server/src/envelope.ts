import type { Context } from "hono";

// Error code table mirroring the live als server. Naming convention is
// UPPER_SNAKE_CASE. Extend here as the server adds more codes; the CLI's
// `from_envelope` table follows this set 1-for-1.
export type ErrorCode =
  | "UNAUTHORIZED"
  | "FORBIDDEN"
  | "NOT_FOUND"
  | "VALIDATION_ERROR"
  | "INVALID_REQUEST"
  | "INVALID_MULTIPART"
  | "ARCHIVE_TOO_LARGE"
  | "ARCHIVE_TOO_MANY_FILES"
  | "ARCHIVE_PATH_TRAVERSAL"
  | "ARCHIVE_PATH_INVALID"
  | "ARCHIVE_PATH_TOO_DEEP"
  | "ARCHIVE_SYMLINK"
  | "ARCHIVE_ENCRYPTED"
  | "ARCHIVE_COMPRESSION_UNSUPPORTED"
  | "ARCHIVE_ZIP64_UNSUPPORTED"
  | "ARCHIVE_MALFORMED"
  | "EXPIRED"
  | "PAIRING_EXPIRED"
  | "PAIRING_ALREADY_AUTHORIZED"
  | "QUOTA_EXCEEDED"
  | "RATE_LIMITED"
  | "INTERNAL_ERROR";

export type HttpStatus =
  | 400
  | 401
  | 402
  | 403
  | 404
  | 409
  | 410
  | 413
  | 422
  | 429
  | 500;

export const ERROR_STATUS: Record<ErrorCode, HttpStatus> = {
  UNAUTHORIZED: 401,
  FORBIDDEN: 403,
  NOT_FOUND: 404,
  VALIDATION_ERROR: 422,
  INVALID_REQUEST: 400,
  INVALID_MULTIPART: 400,
  ARCHIVE_TOO_LARGE: 413,
  ARCHIVE_TOO_MANY_FILES: 400,
  ARCHIVE_PATH_TRAVERSAL: 400,
  ARCHIVE_PATH_INVALID: 400,
  ARCHIVE_PATH_TOO_DEEP: 400,
  ARCHIVE_SYMLINK: 400,
  ARCHIVE_ENCRYPTED: 400,
  ARCHIVE_COMPRESSION_UNSUPPORTED: 400,
  ARCHIVE_ZIP64_UNSUPPORTED: 400,
  ARCHIVE_MALFORMED: 400,
  EXPIRED: 410,
  PAIRING_EXPIRED: 410,
  PAIRING_ALREADY_AUTHORIZED: 409,
  QUOTA_EXCEEDED: 402,
  RATE_LIMITED: 429,
  INTERNAL_ERROR: 500,
};

export interface OkEnvelope<T> {
  success: true;
  data: T;
}

export interface PaginationMeta {
  total: number;
  page: number;
  limit: number;
}

export interface ListEnvelope<T> {
  success: true;
  data: T[];
  meta: PaginationMeta;
}

export interface FailEnvelope {
  success: false;
  error: {
    code: ErrorCode;
    message: string;
  };
}

export const ok = <T>(data: T): OkEnvelope<T> => ({ success: true, data });

export const okList = <T>(data: T[], meta: PaginationMeta): ListEnvelope<T> => ({
  success: true,
  data,
  meta,
});

export const fail = (code: ErrorCode, message: string): FailEnvelope => ({
  success: false,
  error: { code, message },
});

export function errorResponse(c: Context, code: ErrorCode, message: string) {
  return c.json(fail(code, message), ERROR_STATUS[code]);
}
