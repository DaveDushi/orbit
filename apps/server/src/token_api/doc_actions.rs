//! Doc search, recents, favorites, copies, locks, version restores and
//! doc comment thread edits for API tokens and MCP. Every call acts as the token's
//! creator; the page, teamspace and comment repositories decide what that person may see and do.
use orbit_platform::{Id, TimestampMillis};
use serde_json::json;

use super::comments::{comment_error, doc_comment, doc_thread, paragraphs};
use super::{Authorized, DocComment, DocThread, DocView, Result, TokenApiError, hidden_page};
use crate::repositories::api_tokens::ApiTokenScope;
use crate::repositories::page_comments::{CommentBody, PageCommentError};
use crate::repositories::pages::{
    PageError, PageFavorite, PageFavoriteList, PageSearch, RECENT_DEFAULT_LIMIT, RECENT_MAX_LIMIT,
    RecentPageList, revision,
};

impl Authorized<'_> {
    /// Docs whose title or text match `query`, best first.
    pub async fn search_docs(&self, query: &str) -> Result<PageSearch> {
        let pages = self.content(ApiTokenScope::DocsRead, &self.api.pages)?;
        if query.trim().is_empty() || query.chars().count() > 200 || query.len() > 800 {
            return Err(TokenApiError::Invalid(
                "A search query has 1 to 200 characters",
            ));
        }
        let p = self.principal;
        pages
            .search_pages(p.workspace_id, p.actor_id, query)
            .await
            .map_err(hidden_page)
    }

    /// The docs the token's creator opened last, most recent first.
    pub async fn recent_docs(&self, limit: Option<usize>) -> Result<RecentPageList> {
        let pages = self.content(ApiTokenScope::DocsRead, &self.api.pages)?;
        let limit = limit.unwrap_or(RECENT_DEFAULT_LIMIT);
        if !(1..=RECENT_MAX_LIMIT).contains(&limit) {
            return Err(TokenApiError::Invalid("limit must be 1 to 50"));
        }
        let p = self.principal;
        pages
            .recent_pages(p.workspace_id, p.actor_id, limit)
            .await
            .map_err(hidden_page)
    }

    /// The token creator's favorite docs, in their order.
    pub async fn favorite_docs(&self) -> Result<PageFavoriteList> {
        let pages = self.content(ApiTokenScope::DocsRead, &self.api.pages)?;
        let p = self.principal;
        pages
            .favorites(p.workspace_id, p.actor_id)
            .await
            .map_err(hidden_page)
    }

    /// Adds the doc as the last favorite; a favorite stays where it is.
    pub async fn add_favorite_doc(&self, page_id: Id) -> Result<PageFavorite> {
        let pages = self.content(ApiTokenScope::DocsWrite, &self.api.pages)?;
        let p = self.principal;
        pages
            .add_favorite(p.workspace_id, page_id, p.actor_id, TimestampMillis::now())
            .await
            .map_err(hidden_page)
    }

    pub async fn remove_favorite_doc(&self, page_id: Id) -> Result<()> {
        let pages = self.content(ApiTokenScope::DocsWrite, &self.api.pages)?;
        let p = self.principal;
        pages
            .remove_favorite(p.workspace_id, page_id, p.actor_id)
            .await
            .map_err(hidden_page)
    }

    /// Moves a favorite to `position` among the favorites; past the end puts it last.
    pub async fn move_favorite_doc(&self, page_id: Id, position: i64) -> Result<PageFavoriteList> {
        let pages = self.content(ApiTokenScope::DocsWrite, &self.api.pages)?;
        if position < 0 {
            return Err(TokenApiError::Invalid("position must be 0 or more"));
        }
        let p = self.principal;
        pages
            .move_favorite(p.workspace_id, page_id, p.actor_id, position)
            .await
            .map_err(hidden_page)
    }

    /// Copies a doc (with `include_children`, its live sub-docs too) right after it.
    pub async fn duplicate_doc(&self, page_id: Id, include_children: bool) -> Result<DocView> {
        let pages = self.content(ApiTokenScope::DocsWrite, &self.api.pages)?;
        let p = self.principal;
        let copy = pages
            .duplicate_page(
                p.workspace_id,
                page_id,
                p.actor_id,
                include_children,
                self.request_id,
                TimestampMillis::now(),
            )
            .await?;
        self.doc_view(pages, copy).await
    }

    /// Locks or unlocks a doc: a locked doc's title, icon and content cannot change. Any member
    /// who sees the doc may do both, as in the web app.
    pub async fn set_doc_lock(&self, page_id: Id, locked: bool) -> Result<DocView> {
        let pages = self.content(ApiTokenScope::DocsManage, &self.api.pages)?;
        let p = self.principal;
        let page = pages
            .set_lock(
                p.workspace_id,
                page_id,
                p.actor_id,
                locked,
                self.request_id,
                TimestampMillis::now(),
            )
            .await?;
        self.doc_view(pages, page).await
    }

    /// Replaces a doc's content with a saved version's. `expected_version` is the doc's
    /// `version` as read (its [`revision`]).
    pub async fn restore_doc_version(
        &self,
        page_id: Id,
        version_id: Id,
        expected_version: u64,
    ) -> Result<DocView> {
        let pages = self.content(ApiTokenScope::DocsWrite, &self.api.pages)?;
        let p = self.principal;
        let mut current = pages
            .get_page(p.workspace_id, page_id, p.actor_id)
            .await
            .map_err(hidden_page)?;
        let current_revision = revision(current.version, current.updated_at);
        if current_revision != expected_version {
            current.version = current_revision;
            return Err(PageError::VersionConflict {
                current: Box::new(json!(current)),
            }
            .into());
        }
        let page = pages
            .restore_page_version(
                p.workspace_id,
                page_id,
                version_id,
                p.actor_id,
                current.version,
                self.request_id,
                TimestampMillis::now(),
            )
            .await
            .map_err(|error| match error {
                PageError::VersionNotFound => TokenApiError::NotAvailable,
                error => error.into(),
            })?;
        self.doc_view(pages, page).await
    }

    /// Edits a doc comment that the token's creator wrote.
    pub async fn update_doc_comment(
        &self,
        page_id: Id,
        thread_id: Id,
        comment_id: Id,
        markdown: &str,
    ) -> Result<DocComment> {
        if markdown.trim().is_empty() || markdown.len() > 100_000 {
            return Err(TokenApiError::Invalid("Invalid comment body"));
        }
        let repository = self
            .doc_comment_repository(page_id, ApiTokenScope::DocsWrite)
            .await?;
        let blocks = paragraphs(&self.blocks(markdown).await?);
        let body = CommentBody::parse(serde_json::Value::Array(blocks)).map_err(comment_error)?;
        let p = self.principal;
        let comment = repository
            .update_comment(
                p.workspace_id,
                page_id,
                thread_id,
                comment_id,
                p.actor_id,
                body,
                self.request_id,
                TimestampMillis::now(),
            )
            .await
            .map_err(thread_error)?;
        let names = self.member_names().await?;
        Ok(doc_comment(&names, comment))
    }

    /// Deletes a doc comment that the token's creator wrote; it keeps its place, empty.
    pub async fn delete_doc_comment(
        &self,
        page_id: Id,
        thread_id: Id,
        comment_id: Id,
    ) -> Result<()> {
        let repository = self
            .doc_comment_repository(page_id, ApiTokenScope::DocsManage)
            .await?;
        let p = self.principal;
        repository
            .delete_comment(
                p.workspace_id,
                page_id,
                thread_id,
                comment_id,
                p.actor_id,
                self.request_id,
                TimestampMillis::now(),
            )
            .await
            .map_err(thread_error)
    }

    /// Resolves (`true`) or reopens (`false`) a doc comment thread; the same state again is a
    /// no-op.
    pub async fn set_doc_thread_resolved(
        &self,
        page_id: Id,
        thread_id: Id,
        resolved: bool,
    ) -> Result<DocThread> {
        let repository = self
            .doc_comment_repository(page_id, ApiTokenScope::DocsWrite)
            .await?;
        let p = self.principal;
        let thread = repository
            .set_resolved(
                p.workspace_id,
                page_id,
                thread_id,
                p.actor_id,
                resolved,
                self.request_id,
                TimestampMillis::now(),
            )
            .await
            .map_err(thread_error)?;
        let names = self.member_names().await?;
        Ok(doc_thread(&names, thread))
    }

    /// Deletes a thread with its comments: its creator while nobody else commented, or a member
    /// who may moderate comments.
    pub async fn delete_doc_thread(&self, page_id: Id, thread_id: Id) -> Result<()> {
        let repository = self
            .doc_comment_repository(page_id, ApiTokenScope::DocsManage)
            .await?;
        let p = self.principal;
        repository
            .delete_thread(
                p.workspace_id,
                page_id,
                thread_id,
                p.actor_id,
                self.request_id,
                TimestampMillis::now(),
            )
            .await
            .map_err(thread_error)
    }
}

/// [`comment_error`], but telling the refusals of a visible thread apart.
fn thread_error(error: PageCommentError) -> TokenApiError {
    match error {
        PageCommentError::Forbidden => TokenApiError::Forbidden(
            "Only the comment's author may edit or delete it; a thread is deleted by its creator or a member who may moderate comments",
        ),
        PageCommentError::ThreadHasReplies => TokenApiError::Conflict(
            "Others commented in this thread: only a member who may moderate comments can delete it",
        ),
        error => comment_error(error),
    }
}
