#!/usr/bin/env bash
#
# Writes release notes for the commits between the previous release tag and the
# current commit. Used by .github/workflows/release.yml.
#
# usage: changelog.sh <tag> <previous-tag|""> [output]

set -euo pipefail

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
    echo "usage: $0 <tag> <previous-tag|''> [output]" >&2
    exit 2
fi

tag=$1
previous=$2
output=${3:-/dev/stdout}

server=${GITHUB_SERVER_URL:-https://github.com}
repo=${GITHUB_REPOSITORY:?GITHUB_REPOSITORY must be set}

if [ -n "$previous" ]; then
    range="$previous..HEAD"
else
    range="HEAD"
fi

log=$(mktemp)
trap 'rm -f "$log"' EXIT

# Record separator is unit separator (0x1f) so subjects may contain any other
# punctuation. Merge commits are skipped because their subjects repeat branch
# changes.
git log --no-merges \
    --pretty=tformat:'%h%x1f%an%x1f%s' \
    "$range" > "$log"

count=$(wc -l < "$log" | tr -d ' ')

added=()
fixed=()
removed=()
changed=()
other=()

while IFS=$'\x1f' read -r hash author subject; do
    [ -z "$hash" ] && continue
    line="- ${subject} ([${hash}](${server}/${repo}/commit/${hash})) - ${author}"
    case "$subject" in
        Fix* | fix* | Correct* | Prevent* | Avoid* | Handle* | Repair* | Guard*)
            fixed+=("$line")
            ;;
        Remove* | Delete* | Drop*)
            removed+=("$line")
            ;;
        Add* | add* | feat* | Feature* | Implement* | Introduce* | Create* | \
            Enable* | Support* | Render* | Generate* | Define* | Configure* | \
            Choose* | Draw* | Play* | Show*)
            added+=("$line")
            ;;
        Refactor* | refactor* | perf* | Move* | Rename* | Simplify* | Replace* | \
            Use* | Make* | Split* | Merge* | Extract* | Update* | Store* | \
            Cache* | Reduce* | Improve*)
            changed+=("$line")
            ;;
        *)
            other+=("$line")
            ;;
    esac
done < "$log"

print_section() {
    local title=$1
    shift
    if [ "$#" -eq 0 ]; then
        return
    fi
    printf '### %s\n\n' "$title"
    printf '%s\n' "$@"
    printf '\n'
}

{
    printf '# RTS %s\n\n' "$tag"
    printf 'Released %s.\n\n' "$(date -u +%Y-%m-%d)"

    if [ -n "$previous" ]; then
        printf "Previous release: [\`%s\`](%s/%s/releases/tag/%s)\n\n" \
            "$previous" "$server" "$repo" "$previous"
    else
        printf 'This is the first release.\n\n'
    fi

    printf "## What's Changed\n\n"
    if [ "$count" -eq 0 ]; then
        printf 'No commits since the previous release.\n\n'
    else
        printf '%s commit%s.\n\n' "$count" "$([ "$count" -eq 1 ] && echo '' || echo 's')"
        print_section 'Added' "${added[@]}"
        print_section 'Fixed' "${fixed[@]}"
        print_section 'Changed' "${changed[@]}"
        print_section 'Removed' "${removed[@]}"
        print_section 'Other' "${other[@]}"
    fi

    if [ -n "$previous" ]; then
        printf "**Full changelog:** [\`%s...%s\`](%s/%s/compare/%s...%s)\n" \
            "$previous" "$tag" "$server" "$repo" "$previous" "$tag"
    fi
} > "$output"
