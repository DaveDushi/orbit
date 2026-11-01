import { Navigate, useLocation } from 'react-router'

/** A page that has a new address: goes there and keeps the query and the hash. */
export function MovedTo({ path }: { path: string }) {
  const { search, hash } = useLocation()
  return <Navigate to={{ pathname: path, search, hash }} replace />
}
