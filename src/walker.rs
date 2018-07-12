use Arena;

// Taken and adapted from https://github.com/bluss/petgraph
/// A walker is a traversal state, but where part of the traversal
/// information is supplied manually to each next call.
///
/// This for example allows graph traversals that don't hold a borrow of the
/// graph they are traversing.
pub trait Walker<T> {
    type Item;
    /// Advance to the next item
    fn walk_next(&mut self, context: &Arena<T>) -> Option<Self::Item>;

    /// Create an iterator out of the walker and given `context`.
    fn iter<'a>(self, arena: &'a Arena<T>) -> WalkerIter<'a, Self, T>
        where Self: Sized
    {
        WalkerIter {
            walker: self,
            arena,
        }
    }
}

/// A walker and its context wrapped into an iterator.
#[derive(Clone, Debug)]
pub struct WalkerIter<'a, W, T: 'a> {
    walker: W,
    arena: &'a Arena<T>,
}

impl<'a, W, T> WalkerIter<'a, W, T>
    where W: Walker<T>
{
    pub fn arena(&self) -> &'a Arena<T> {
        self.arena
    }

    pub fn inner_ref(&self) -> &W {
        &self.walker
    }

    pub fn inner_mut(&mut self) -> &mut W {
        &mut self.walker
    }
}

impl<'a, W, T> Iterator for WalkerIter<'a, W, T>
    where W: Walker<T>,
{
    type Item = W::Item;
    fn next(&mut self) -> Option<Self::Item> {
        self.walker.walk_next(self.arena)
    }
}
