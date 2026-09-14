/// Everything the command modules need from the global flags.
///
/// Passed by reference to every `run()` so adding a global flag does not mean
/// touching nineteen signatures again.
pub struct Ctx {
    pub json: bool,
    pub yes: bool,
    pub page: Option<u32>,
    pub limit: Option<u32>,
}

impl Ctx {
    /// True when the user asked for a specific slice of a list.
    pub fn has_paging(&self) -> bool {
        self.page.is_some() || self.limit.is_some()
    }

    /// Query pairs for the majority of routes, which read `page` + `pageSize`.
    pub fn paging(&self) -> Vec<(&'static str, String)> {
        self.paging_named("pageSize")
    }

    /// Query pairs for routes that spell the page size `limit`
    /// (datasets, alert history, GDPR, contributions).
    pub fn paging_limit(&self) -> Vec<(&'static str, String)> {
        self.paging_named("limit")
    }

    fn paging_named(&self, size_key: &'static str) -> Vec<(&'static str, String)> {
        let mut q = Vec::new();
        if let Some(p) = self.page {
            q.push(("page", p.to_string()));
        }
        if let Some(l) = self.limit {
            q.push((size_key, l.to_string()));
        }
        q
    }

    /// Merge the paging flags into a command's own filters.
    pub fn with_paging(&self, mut own: Vec<(&'static str, String)>) -> Vec<(&'static str, String)> {
        own.extend(self.paging());
        own
    }

    /// As `with_paging`, for `limit`-style routes.
    pub fn with_paging_limit(
        &self,
        mut own: Vec<(&'static str, String)>,
    ) -> Vec<(&'static str, String)> {
        own.extend(self.paging_limit());
        own
    }
}
