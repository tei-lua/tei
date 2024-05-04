use super::context::Visitor;

pub unsafe trait Managed {
    fn needs_trace() -> bool
    where
        Self: Sized;

    fn trace(&self, _cc: &Visitor) {}
}
