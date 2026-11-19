//! The HTML body of a customer email, cleaned with `ammonia` before it is stored: an allow list of
//! tags and attributes (no script, style, form, iframe, object or event attributes), links that
//! open in a new tab without a referrer, and `cid:` images rewritten to the task attachment that
//! holds them. The web app shows the result in a sandboxed iframe with a strict CSP that blocks
//! external images until the reader selects "Show images".

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

/// Tags kept from an email. Others are removed and their text kept; `CLEAN_CONTENT` ones go with
/// their content.
const TAGS: &[&str] = &[
    "a",
    "abbr",
    "b",
    "blockquote",
    "br",
    "caption",
    "center",
    "code",
    "col",
    "colgroup",
    "dd",
    "del",
    "div",
    "dl",
    "dt",
    "em",
    "font",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "i",
    "img",
    "ins",
    "li",
    "ol",
    "p",
    "pre",
    "q",
    "s",
    "small",
    "span",
    "strike",
    "strong",
    "sub",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "u",
    "ul",
];
const CLEAN_CONTENT: &[&str] = &[
    "script", "style", "title", "head", "noscript", "template", "iframe", "object", "embed",
    "form", "textarea", "select", "button", "svg", "math",
];

/// Cleans `html`. `inline_images` maps a Content-ID (without angle brackets) to the URL of the
/// attachment that holds it; a `cid:` image with no attachment loses its source.
#[must_use]
pub fn sanitize(html: &str, inline_images: HashMap<String, String>) -> String {
    let tag_attributes: HashMap<&str, HashSet<&str>> = [
        ("a", ["href", "title"].into_iter().collect()),
        (
            "img",
            ["src", "alt", "width", "height", "title"]
                .into_iter()
                .collect(),
        ),
        (
            "td",
            ["colspan", "rowspan", "align", "valign", "width"]
                .into_iter()
                .collect(),
        ),
        (
            "th",
            ["colspan", "rowspan", "align", "valign", "width"]
                .into_iter()
                .collect(),
        ),
        (
            "table",
            ["width", "align", "cellpadding", "cellspacing", "border"]
                .into_iter()
                .collect(),
        ),
        ("col", ["span", "width"].into_iter().collect()),
        ("p", ["align"].into_iter().collect()),
        ("div", ["align"].into_iter().collect()),
        ("font", ["color", "size"].into_iter().collect()),
        ("ol", ["start"].into_iter().collect()),
    ]
    .into_iter()
    .collect();
    let mut builder = ammonia::Builder::default();
    builder
        .tags(TAGS.iter().copied().collect())
        .clean_content_tags(CLEAN_CONTENT.iter().copied().collect())
        .tag_attributes(tag_attributes)
        .generic_attributes(["dir", "lang"].into_iter().collect())
        .url_schemes(["http", "https", "mailto", "cid"].into_iter().collect())
        // Relative URLs pass here and the filter below drops them: it runs after this check and
        // rewrites `cid:` images to relative attachment URLs, which `Deny` would remove again.
        .url_relative(ammonia::UrlRelative::PassThrough)
        .link_rel(Some("noopener noreferrer nofollow"))
        .set_tag_attribute_value("a", "target", "_blank")
        .strip_comments(true)
        .attribute_filter(move |element, attribute, value| {
            if element == "img" && attribute == "src" {
                return image_source(value, &inline_images);
            }
            if attribute == "href" {
                let lower = value.trim_start().to_ascii_lowercase();
                let absolute = ["https://", "http://", "mailto:"]
                    .iter()
                    .any(|scheme| lower.starts_with(scheme));
                return absolute.then_some(Cow::Borrowed(value));
            }
            Some(Cow::Borrowed(value))
        });
    builder.clean(html).to_string()
}

/// `cid:` sources become the attachment's URL; http(s) images stay (the iframe's CSP blocks them
/// until the reader allows them); anything else is dropped.
fn image_source<'u>(
    value: &'u str,
    inline_images: &HashMap<String, String>,
) -> Option<Cow<'u, str>> {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("cid:") {
        let id = super::inbound::bare_id(&trimmed[4..]);
        return inline_images.get(&id).map(|url| Cow::Owned(url.clone()));
    }
    (lower.starts_with("https://") || lower.starts_with("http://")).then_some(Cow::Borrowed(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(html: &str) -> String {
        sanitize(html, HashMap::new())
    }

    #[test]
    fn scripts_styles_forms_and_frames_are_removed_with_their_content() {
        let out = clean(
            "<p>Hi</p><script>alert(1)</script><style>p{color:red}</style>\
             <form action=\"https://evil.test\"><input name=\"x\"><button>Go</button></form>\
             <iframe src=\"https://evil.test\"></iframe><object data=\"x\"></object>\
             <svg><script>alert(2)</script></svg>",
        );
        assert_eq!(out, "<p>Hi</p>");
    }

    #[test]
    fn event_handlers_style_attributes_and_javascript_urls_are_removed() {
        let out = clean(
            "<p onclick=\"alert(1)\" style=\"background:url(https://t.test/p.gif)\">x</p>\
             <a href=\"javascript:alert(1)\">a</a><a href=\"JaVaScRiPt:alert(1)\">b</a>\
             <img src=\"x\" onerror=\"alert(1)\"><a href=\"data:text/html,<script>alert(1)</script>\">c</a>",
        );
        for bad in [
            "onclick",
            "style",
            "javascript",
            "onerror",
            "data:",
            "alert",
        ] {
            assert!(!out.to_ascii_lowercase().contains(bad), "{bad} in {out}");
        }
    }

    #[test]
    fn links_open_in_a_new_tab_without_referrer() {
        let out = clean("<a href=\"https://example.com/x\">x</a>");
        assert!(out.contains("href=\"https://example.com/x\""), "{out}");
        assert!(out.contains("target=\"_blank\""), "{out}");
        assert!(
            out.contains("rel=\"noopener noreferrer nofollow\""),
            "{out}"
        );
    }

    #[test]
    fn cid_images_point_to_their_attachment_and_unknown_ones_lose_the_source() {
        let images = HashMap::from([(
            "logo@x".to_owned(),
            "/api/v1/workspaces/w/tasks/t/attachments/a/download".to_owned(),
        )]);
        let out = sanitize(
            "<img src=\"cid:logo@x\" alt=\"Logo\"><img src=\"CID:<missing@x>\">\
             <img src=\"https://tracker.test/pixel.gif\"><img src=\"/relative.png\"><a href=\"/admin\">r</a>",
            images,
        );
        assert!(
            out.contains("src=\"/api/v1/workspaces/w/tasks/t/attachments/a/download\""),
            "{out}"
        );
        assert!(!out.contains("missing"), "{out}");
        assert!(out.contains("https://tracker.test/pixel.gif"), "{out}");
        assert!(!out.contains("relative.png"), "{out}");
        assert!(!out.contains("/admin"), "{out}");
    }

    #[test]
    fn broken_markup_cannot_escape_the_tree() {
        let out = clean("<p>a</p></div></body><img src=x onerror=alert(1)//<p>b");
        assert!(!out.contains("onerror"), "{out}");
        let out = clean("<<script>script>alert(1)<</script>/script>");
        assert!(!out.contains("<script"), "{out}");
    }
}
