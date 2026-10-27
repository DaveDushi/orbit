#!/usr/bin/env bash
# Creates the Orbit Cloud products and prices in Stripe, then prints ORBIT_STRIPE_PRICES.
# Run it once with a test key and once with the live key: each run finds what exists by product id
# and price lookup key, so running it again creates nothing new. It renames products whose name
# differs, since Orbit shows the product name on the billing page.
#
#   STRIPE_SECRET_KEY=sk_test_… scripts/stripe-setup.sh
#   STRIPE_SECRET_KEY=sk_live_… ORBIT_WEBHOOK_URL=https://<origin>/api/v1/billing/stripe/webhook scripts/stripe-setup.sh
set -euo pipefail

: "${STRIPE_SECRET_KEY:?set STRIPE_SECRET_KEY}"
api=https://api.stripe.com/v1

stripe() {
    local method=$1 path=$2
    shift 2
    curl -sS -X "$method" -u "$STRIPE_SECRET_KEY:" "$api$path" "$@"
}

# plan key, product id, product name, amount in cents, interval
plans=(
    "team-monthly orbit_team Team 1000 month"
    "team-annual orbit_team Team 10000 year"
    "business-monthly orbit_business Business 2900 month"
    "business-annual orbit_business Business 29000 year"
)

prices=()
for line in "${plans[@]}"; do
    read -r key product name amount interval <<<"$line"
    existing=$(stripe GET "/products/$product")
    if jq -e '.error' >/dev/null <<<"$existing"; then
        stripe POST /products -d id="$product" -d name="$name" \
            -d "metadata[orbit_plan]=${key%-*}" | jq -er '.id' >/dev/null
        echo "created product $product" >&2
    elif [ "$(jq -r '.name' <<<"$existing")" != "$name" ]; then
        stripe POST "/products/$product" -d name="$name" | jq -er '.id' >/dev/null
        echo "renamed product $product to $name" >&2
    fi
    lookup="orbit_${key//-/_}"
    price=$(stripe GET /prices -G -d "lookup_keys[]=$lookup" -d active=true | jq -r '.data[0].id // empty')
    if [ -z "$price" ]; then
        price=$(stripe POST /prices -d product="$product" -d currency=usd -d unit_amount="$amount" \
            -d "recurring[interval]=$interval" -d lookup_key="$lookup" | jq -er '.id')
        echo "created price $lookup ($price)" >&2
    fi
    prices+=("$key:$price")
done

if [ -n "${ORBIT_WEBHOOK_URL:-}" ]; then
    existing=$(stripe GET /webhook_endpoints -G -d limit=100 |
        jq -r --arg url "$ORBIT_WEBHOOK_URL" '.data[] | select(.url == $url) | .id' | head -1)
    if [ -n "$existing" ]; then
        echo "webhook $existing already exists; its signing secret is in the Stripe Dashboard" >&2
    else
        secret=$(stripe POST /webhook_endpoints -d url="$ORBIT_WEBHOOK_URL" \
            -d "enabled_events[]=checkout.session.completed" \
            -d "enabled_events[]=customer.subscription.created" \
            -d "enabled_events[]=customer.subscription.updated" \
            -d "enabled_events[]=customer.subscription.deleted" | jq -er '.secret')
        echo "ORBIT_STRIPE_WEBHOOK_SECRET=$secret"
    fi
fi

(IFS=,; echo "ORBIT_STRIPE_PRICES=${prices[*]}")
