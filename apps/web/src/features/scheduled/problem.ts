import { ApiProblem } from '@orbit/api/problem'

/** The server's reason for a refusal, or `fallback`. */
export function problemDetail(error: unknown, fallback: string): string {
  return error instanceof ApiProblem && error.detail ? error.detail : fallback
}
