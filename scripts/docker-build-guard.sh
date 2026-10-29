#!/usr/bin/env bash
# Installed only on the shared build VM as /usr/local/bin/docker.
set -euo pipefail

case "${1:-} ${2:-}" in
    'build '*)
        shift
        load=(--load)
        ;;
    'buildx build')
        shift 2
        load=()
        ;;
    *)
        exec /usr/bin/docker "$@"
        ;;
esac

# Buildx flags may be repeated; reject *any* conflicting builder, not just the
# last one. Do not interpret flags after the command's "--" delimiter.
args=("$@")
for (( i=0; i<${#args[@]}; i++ )); do
    case "${args[i]}" in
        --) break ;;
        --output|--output=*|-o|-o?*|--push|--push=*|--load|--load=*)
            # Preserve explicit exporters rather than forcing a second export
            # to the local image store (or conflicting with --load=false).
            load=()
            continue
            ;;
        --builder=*) builder=${args[i]#--builder=} ;;
        --builder)
            i=$((i + 1))
            builder=${args[i]:-}
            ;;
        *) continue ;;
    esac
    if [[ "$builder" != orbit-limited ]]; then
        printf 'docker build guard: --builder must be orbit-limited on this VM; see deploy/README.md.\n' >&2
        exit 64
    fi
done

export BUILDX_BUILDER=orbit-limited
exec /usr/local/bin/orbit-build /usr/bin/docker buildx build --builder=orbit-limited "${load[@]}" "$@"
