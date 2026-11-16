import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileRoutes, routes, sweepVerdict } from './routes'

// A router like public_api.rs, plus a planted route that no auth layer covers.
const planted = `
pub const BASE_PATH: &str = "/api/public/v1";
pub fn router(state: S) -> Router {
    let workspace = Router::new()
        .route("/tasks/{task}", get(get_task).patch(update_task))
        .route("/leak", get(leak));
    let routes = Router::new()
        .route("/me", get(me))
        .nest("/workspaces/{workspace_id}", workspace)
        .route_layer(middleware::from_fn_with_state(state.clone(), authenticate));
    Router::new().nest(BASE_PATH, routes)
}
pub fn other() -> Router {
    Router::new().route("/api/v1/plain", post(plain))
}
`

describe('routes', () => {
  test('nested routers get the prefixes of every .nest that mounts them', () => {
    const { routes, unresolved } = fileRoutes(planted, 'planted.rs')
    expect(unresolved).toEqual([])
    expect(routes.map((route) => `${route.method} ${route.path}`).sort()).toEqual([
      'GET /api/public/v1/me',
      'GET /api/public/v1/workspaces/{workspace_id}/leak',
      'GET /api/public/v1/workspaces/{workspace_id}/tasks/{task}',
      'PATCH /api/public/v1/workspaces/{workspace_id}/tasks/{task}',
      'POST /api/v1/plain',
    ])
  })

  test('public_api.rs routes carry /api/public/v1, none is listed without it', () => {
    const file = 'apps/server/src/public_api.rs'
    const { routes: scanned } = fileRoutes(readFileSync(join(import.meta.dir, '../..', file), 'utf8'), file)
    expect(scanned.length).toBeGreaterThan(10)
    for (const route of scanned) expect(route.path).toStartWith('/api/public/v1/')
    expect(routes().filter((route) => route.file === file && !route.path.startsWith('/api/public/v1/'))).toEqual([])
  })

  test('the sweep flags a protected or planted route that answers without a session', () => {
    expect(sweepVerdict({ method: 'GET', path: '/api/public/v1/workspaces/{workspace_id}/leak' }, 200)).toBe('open')
    expect(sweepVerdict({ method: 'GET', path: '/api/public/v1/me' }, 200)).toBe('open')
    expect(sweepVerdict({ method: 'GET', path: '/api/public/v1/me' }, 401)).toBe('refused')
    expect(sweepVerdict({ method: 'POST', path: '/api/v1/auth/login' }, 200)).toBe('public')
    expect(sweepVerdict({ method: 'GET', path: '/api/v1/admin/users' }, 404)).toBe('other-4xx')
  })
})
