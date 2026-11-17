/**
 * 105 tools of four fake connected services, as Orbit's `GET /agent-tools` lists them (`<slug>__<tool>`): TikTok Ads
 * (40, like the `tt-ads-mcp-layer` server), TikTok organic (15), Shopify (25) and Cloudflare (25). A tool whose name
 * starts with a write verb needs the driver's approval (`always`), as Orbit marks MCP tools that are not read-only.
 * `mcpResult` answers each read with a small JSON result.
 */

import type { IntegrationTool } from "../../apps/agent/src/integrations.ts";

type Props = Record<string, string>;

const WRITE = /^(create|update|delete|pause|enable|disable|upload|set|add|remove|purge|publish|reply|cancel|refund|fulfill|block|archive)_/;

/** A JSON Schema object from `name: "type description"` pairs; a `*` before the type marks a required one. */
function schema(props: Props): IntegrationTool["input_schema"] {
	const properties: Record<string, { type: string; description: string }> = {};
	const required: string[] = [];
	for (const [name, spec] of Object.entries(props)) {
		const [type, ...words] = spec.replace(/^\*/, "").split(" ");
		properties[name] = { type, description: words.join(" ") };
		if (spec.startsWith("*")) required.push(name);
	}
	return { type: "object", properties, required };
}

const ADV = { advertiser_id: "*string The advertiser account id" };

const TT_ADS: [string, string, Props][] = [
	["get_advertisers", "List the TikTok ad accounts (advertisers) this connection can manage, with currency and time zone.", {}],
	["get_advertiser_info", "Read one advertiser's details: name, balance, status, industry.", ADV],
	["get_campaigns", "List the advertiser's ad campaigns with status, objective and daily or lifetime budget.", { ...ADV, status: "string ENABLE, DISABLE or DELETE", page: "number" }],
	["get_campaign", "Read one campaign: name, objective, budget mode, budget, status.", { ...ADV, campaign_id: "*string" }],
	["create_campaign", "Create an ad campaign with an objective (traffic, conversions, reach) and a budget.", { ...ADV, name: "*string", objective: "*string", budget: "number", budget_mode: "string" }],
	["update_campaign", "Change a campaign's name or objective settings.", { ...ADV, campaign_id: "*string", name: "string" }],
	["update_campaign_budget", "Change the daily or lifetime budget of one campaign.", { ...ADV, campaign_id: "*string", budget: "*number New budget in the account currency" }],
	["update_campaign_status", "Turn campaigns on or off (ENABLE, DISABLE) or delete them.", { ...ADV, campaign_ids: "*array", status: "*string ENABLE, DISABLE or DELETE" }],
	["get_ad_groups", "List the ad groups of the advertiser or of one campaign, with targeting, bid and schedule.", { ...ADV, campaign_id: "string" }],
	["get_ad_group", "Read one ad group: placement, targeting, bid strategy, budget and schedule.", { ...ADV, adgroup_id: "*string" }],
	["create_ad_group", "Create an ad group in a campaign with targeting, placement, bid and budget.", { ...ADV, campaign_id: "*string", name: "*string", budget: "number" }],
	["update_ad_group", "Change an ad group's targeting, bid, budget or schedule.", { ...ADV, adgroup_id: "*string", budget: "number", bid: "number" }],
	["update_ad_group_status", "Turn ad groups on or off (ENABLE, DISABLE) or delete them.", { ...ADV, adgroup_ids: "*array", status: "*string ENABLE, DISABLE or DELETE" }],
	["get_ads", "List the ads of an ad group or campaign with their creative and status.", { ...ADV, adgroup_id: "string" }],
	["get_ad", "Read one ad: its video, text, call to action and landing page.", { ...ADV, ad_id: "*string" }],
	["create_ad", "Create an ad in an ad group from a video or image, text and a call to action.", { ...ADV, adgroup_id: "*string", video_id: "string", ad_text: "*string" }],
	["update_ad", "Change an ad's text, call to action or landing page URL.", { ...ADV, ad_id: "*string", ad_text: "string" }],
	["update_ad_status", "Turn ads on or off (ENABLE, DISABLE) or delete them.", { ...ADV, ad_ids: "*array", status: "*string" }],
	["get_report", "Run a performance report: spend, impressions, clicks, CTR, CPC, conversions and ROAS by day, campaign, ad group or ad.", { ...ADV, level: "string campaign, adgroup or ad", start_date: "*string YYYY-MM-DD", end_date: "*string YYYY-MM-DD", metrics: "array e.g. spend, impressions, clicks" }],
	["get_audience_report", "Report the audience of the ads by age, gender, country or device.", { ...ADV, start_date: "*string", end_date: "*string", dimension: "*string age, gender, country or device" }],
	["get_custom_audiences", "List the custom audiences (customer files, website visitors, engagement).", ADV],
	["create_custom_audience", "Create a custom audience from a customer file or from people who engaged.", { ...ADV, name: "*string", source: "*string" }],
	["create_lookalike_audience", "Create a lookalike audience from a custom audience, in some countries.", { ...ADV, source_audience_id: "*string", countries: "*array" }],
	["delete_audience", "Delete custom or lookalike audiences.", { ...ADV, audience_ids: "*array" }],
	["get_pixels", "List the TikTok pixels that track website events, with their status.", ADV],
	["get_pixel_events", "Read the events a pixel received: page views, add to cart, purchases.", { ...ADV, pixel_id: "*string", start_date: "string" }],
	["create_pixel", "Create a TikTok pixel for a website.", { ...ADV, name: "*string" }],
	["get_videos", "List the videos in the ad account's creative library.", ADV],
	["upload_video", "Upload a video to the ad account's creative library from a URL.", { ...ADV, video_url: "*string" }],
	["get_images", "List the images in the ad account's creative library.", ADV],
	["upload_image", "Upload an image to the ad account's creative library from a URL.", { ...ADV, image_url: "*string" }],
	["get_spark_ads_posts", "List the organic posts authorized for Spark Ads.", ADV],
	["get_billing", "Read the account's balance, invoices and payment method.", ADV],
	["get_budget_recommendations", "Recommended budgets for campaigns that are limited by budget.", ADV],
	["get_bid_recommendations", "Recommended bids for ad groups.", { ...ADV, adgroup_id: "string" }],
	["get_targeting_options", "List interest, behavior and region categories for ad group targeting.", { ...ADV, type: "string interest, behavior or region" }],
	["get_rules", "List the automated rules (for example pause an ad when the cost per result is too high).", ADV],
	["create_rule", "Create an automated rule that pauses or changes ads when a metric crosses a limit.", { ...ADV, name: "*string", condition: "*string", action: "*string" }],
	["get_comments", "List the comments people wrote on the ads.", { ...ADV, ad_id: "string" }],
	["reply_comment", "Reply to a comment on an ad.", { ...ADV, comment_id: "*string", text: "*string" }],
];

const TIKTOK: [string, string, Props][] = [
	["get_user_info", "Read the brand's TikTok account: name, followers, following, likes, video count.", {}],
	["list_videos", "List the brand's own TikTok videos (organic posts) with views, likes, comments and shares.", { max_count: "number", cursor: "number" }],
	["query_videos", "Read chosen videos of the brand's account by id, with their statistics.", { video_ids: "*array" }],
	["get_video_insights", "Read one video's insights: reach, watch time, average view duration, traffic sources.", { video_id: "*string" }],
	["get_account_insights", "Read the account's daily followers, profile views and video views.", { start_date: "string", end_date: "string" }],
	["get_follower_demographics", "Read the followers by country, gender and age.", {}],
	["list_comments", "List the comments on one of the brand's videos.", { video_id: "*string" }],
	["reply_comment", "Reply to a comment on one of the brand's videos.", { video_id: "*string", comment_id: "*string", text: "*string" }],
	["delete_comment", "Delete a comment on one of the brand's videos.", { comment_id: "*string" }],
	["publish_video", "Post a video to the brand's TikTok account from a URL, with a caption.", { video_url: "*string", caption: "*string" }],
	["publish_photo", "Post photos to the brand's TikTok account, with a caption.", { photo_urls: "*array", caption: "*string" }],
	["get_publish_status", "Read the status of a post that is being published.", { publish_id: "*string" }],
	["get_creator_info", "Read what the account may post: privacy options, maximum video length.", {}],
	["list_mentions", "List videos of other accounts that mention or tag the brand.", {}],
	["get_trending_hashtags", "List hashtags that trend now in a country.", { country: "string" }],
];

const SHOPIFY: [string, string, Props][] = [
	["list_orders", "List the store's orders, newest first, by status or date.", { status: "string open, closed, cancelled or any", created_at_min: "string" }],
	["get_order", "Read one order by its number or id: customer, items, payment and fulfillment status.", { order: "*string Order number (1042) or id" }],
	["cancel_order", "Cancel an order, optionally with a refund and a restock.", { order_id: "*string", reason: "string" }],
	["refund_order", "Refund an order fully or in part.", { order_id: "*string", amount: "number" }],
	["fulfill_order", "Mark an order fulfilled with a tracking number.", { order_id: "*string", tracking_number: "string" }],
	["list_products", "List the store's products with price, stock and status.", { query: "string" }],
	["get_product", "Read one product: variants, prices, stock and images.", { product_id: "*string" }],
	["create_product", "Create a product with a title, price and description.", { title: "*string", price: "*number" }],
	["update_product", "Change a product's title, description, price or status.", { product_id: "*string", title: "string", price: "number" }],
	["delete_product", "Delete a product.", { product_id: "*string" }],
	["set_inventory", "Set the stock of a product variant at a location.", { variant_id: "*string", quantity: "*number" }],
	["list_customers", "List or search the store's customers.", { query: "string" }],
	["get_customer", "Read one customer: contact, orders and total spent.", { customer_id: "*string" }],
	["update_customer", "Change a customer's contact details or tags.", { customer_id: "*string", tags: "array" }],
	["list_collections", "List the store's product collections.", {}],
	["add_product_to_collection", "Add a product to a collection.", { product_id: "*string", collection_id: "*string" }],
	["list_discounts", "List the store's discount codes and automatic discounts.", {}],
	["create_discount", "Create a discount code (percent or amount) for some days.", { code: "*string", percent: "number", ends_at: "string" }],
	["get_sales_report", "Report sales, orders and average order value by day or product.", { start_date: "*string", end_date: "*string", by: "string day or product" }],
	["list_abandoned_checkouts", "List checkouts customers did not finish.", {}],
	["list_blog_articles", "List the articles of the store's blog.", { blog_id: "string" }],
	["create_blog_article", "Write a new blog article.", { blog_id: "*string", title: "*string", body_html: "*string" }],
	["update_blog_article", "Change a blog article's title, text or image.", { article_id: "*string", body_html: "string" }],
	["get_shop", "Read the store's name, domain, plan, currency and time zone.", {}],
	["list_locations", "List the store's locations and warehouses.", {}],
];

const CLOUDFLARE: [string, string, Props][] = [
	["list_zones", "List the domains (zones) of the Cloudflare account with their status and plan.", { name: "string Domain name" }],
	["get_zone", "Read one zone's settings: plan, name servers, status.", { zone_id: "*string" }],
	["list_dns_records", "List the DNS records of a zone (A, AAAA, CNAME, MX, TXT).", { zone_id: "*string", type: "string" }],
	["create_dns_record", "Add a DNS record to a zone.", { zone_id: "*string", type: "*string", name: "*string", content: "*string" }],
	["update_dns_record", "Change a DNS record's content, TTL or proxy setting.", { zone_id: "*string", record_id: "*string", content: "string" }],
	["delete_dns_record", "Delete a DNS record.", { zone_id: "*string", record_id: "*string" }],
	["purge_cache", "Purge the cache of a zone: everything, or chosen URLs.", { zone_id: "*string", files: "array URLs; empty for everything" }],
	["get_analytics", "Read a zone's traffic: requests, bandwidth, cached share, threats, by hour or day.", { zone_id: "*string", since: "string" }],
	["list_firewall_rules", "List a zone's firewall (WAF) rules.", { zone_id: "*string" }],
	["create_firewall_rule", "Add a firewall rule that blocks or challenges matching requests.", { zone_id: "*string", expression: "*string", action: "*string" }],
	["block_ip", "Block an IP address or range for the whole account.", { ip: "*string", note: "string" }],
	["list_workers", "List the account's Workers scripts.", {}],
	["get_worker", "Read a Worker's code, routes and bindings.", { name: "*string" }],
	["list_pages_projects", "List the Cloudflare Pages projects with their latest deployment.", {}],
	["list_pages_deployments", "List the deployments of a Pages project.", { project: "*string" }],
	["list_r2_buckets", "List the account's R2 storage buckets.", {}],
	["list_r2_objects", "List objects in an R2 bucket by prefix.", { bucket: "*string", prefix: "string" }],
	["delete_r2_object", "Delete an object from an R2 bucket.", { bucket: "*string", key: "*string" }],
	["get_ssl_settings", "Read a zone's SSL/TLS mode and certificate status.", { zone_id: "*string" }],
	["set_ssl_mode", "Set a zone's SSL/TLS mode (off, flexible, full, strict).", { zone_id: "*string", mode: "*string" }],
	["list_page_rules", "List a zone's page rules (redirects, cache settings).", { zone_id: "*string" }],
	["create_redirect", "Add a redirect from one URL pattern to another.", { zone_id: "*string", from: "*string", to: "*string" }],
	["get_audit_logs", "Read the account's audit log: who changed what and when.", { since: "string" }],
	["list_tunnels", "List the account's Cloudflare Tunnels and their status.", {}],
	["get_dns_analytics", "Report DNS queries of a zone by record name and response code.", { zone_id: "*string" }],
];

function service(slug: string, integration: string, tools: [string, string, Props][]): IntegrationTool[] {
	return tools.map(([name, description, props]) => ({ name: `${slug}__${name}`, integration, description, input_schema: schema(props), approval: WRITE.test(name) ? "always" : "never" }));
}

export const MCP_TOOLS: IntegrationTool[] = [
	...service("tt_ads", "TikTok Ads", TT_ADS),
	...service("tiktok", "TikTok", TIKTOK),
	...service("shopify", "Shopify", SHOPIFY),
	...service("cloudflare", "Cloudflare", CLOUDFLARE),
];

/** A read's result: fixed data for the tools the cases use, an empty list for the others. */
export function mcpResult(tool: string, args: Record<string, unknown>): unknown {
	switch (tool) {
		case "tt_ads__get_advertisers":
			return { list: [{ advertiser_id: "7001", name: "Wellrox", currency: "EUR" }] };
		case "tt_ads__get_campaigns":
			return {
				list: [
					{ campaign_id: "cmp_1001", name: "Always on", status: "ENABLE", budget: 80, spend_today: 41 },
					{ campaign_id: "cmp_1002", name: "Black Friday", status: "ENABLE", budget: 300, spend_today: 312 },
					{ campaign_id: "cmp_1003", name: "Retargeting", status: "DISABLE", budget: 50, spend_today: 0 },
				],
			};
		case "tt_ads__get_ad_groups":
			return { list: [{ adgroup_id: "ag_2001", name: "Retargeting EU", campaign_id: "cmp_1003", status: "ENABLE" }, { adgroup_id: "ag_2002", name: "Lookalike DE", campaign_id: "cmp_1001", status: "ENABLE" }] };
		case "tt_ads__get_report":
			return { rows: [{ stat_time_day: args.start_date ?? "2026-11-14", spend: 353.4, impressions: 120_400, clicks: 2_310, conversions: 41 }] };
		case "tiktok__list_videos":
			return { videos: [{ id: "v1", title: "Autumn drop", view_count: 48_000 }, { id: "v2", title: "Behind the scenes", view_count: 12_500 }] };
		case "shopify__get_order":
			return { order: { name: `#${args.order}`, financial_status: "paid", fulfillment_status: "unfulfilled", total_price: "129.00" } };
		case "cloudflare__list_zones":
			return { result: [{ id: "zone_9f1", name: "orbit.test", status: "active" }] };
		default:
			return { list: [] };
	}
}
