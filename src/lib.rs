//! # Tree based tree data structure
//!
//! This arena tree structure is using just a single `Vec` and numerical identifiers (indices in the vector) instead of
//! reference counted pointers like. This means there is no `RefCell` and mutability is handled in a way much more
//! idiomatic to Rust through unique (&mut) access to the arena. The tree can be sent or shared across threads like a `Vec`.
//! This enables general multiprocessing support like parallel tree traversals.
//!
//! # Example usage
//! ```
//! use indextree::Tree;
//!
//! // Create a new arena
//! let arena = &mut Tree::new();
//!
//! // Add some new nodes to the arena
//! let a = arena.new_node(1);
//! let b = arena.new_node(2);
//!
//! // Append b to a
//! a.append(b, arena);
//! assert_eq!(b.ancestors(arena).into_iter().count(), 2);
//! ```
#[cfg(feature = "deser")]
extern crate serde;
#[cfg(feature = "deser")]
#[macro_use]
extern crate serde_derive;
#[cfg(feature = "par_iter")]
extern crate rayon;

#[cfg(feature = "par_iter")]
use rayon::prelude::*;
use std::ops::{Index, IndexMut};
use std::{fmt, mem};
use std::collections::VecDeque;
pub use walker::{Walker, WalkerIter};

pub mod walker;

#[derive(PartialEq, Eq, PartialOrd, Ord, Copy, Clone, Debug, Hash)]
#[cfg_attr(feature = "deser", derive(Deserialize, Serialize))]
/// A node identifier within a particular `Tree`
pub struct NodeId {
    index: usize,
}

#[derive(PartialEq, Clone, Debug)]
#[cfg_attr(feature = "deser", derive(Deserialize, Serialize))]
/// A node within a particular `Tree`
pub struct Node<T> {
    // Keep these private (with read-only accessors) so that we can keep them consistent.
    // E.g. the parent of a node’s child is that node.
    parent: Option<NodeId>,
    previous_sibling: Option<NodeId>,
    next_sibling: Option<NodeId>,
    first_child: Option<NodeId>,
    last_child: Option<NodeId>,

    /// The actual data which will be stored within the tree
    pub data: T,
}

impl<T> fmt::Display for Node<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Parent: {:?}, ", self.parent)?;
        write!(f, "Previous sibling: {:?}, ", self.previous_sibling)?;
        write!(f, "Next sibling: {:?}, ", self.next_sibling)?;
        write!(f, "First child: {:?}, ", self.first_child)?;
        write!(f, "Last child: {:?}", self.last_child)
    }
}

#[derive(PartialEq, Clone, Debug)]
#[cfg_attr(feature = "deser", derive(Deserialize, Serialize))]
/// An `Tree` structure containing certain Nodes
pub struct Tree<T> {
    nodes: Vec<Node<T>>,
    orphaned_nodes: VecDeque<NodeId>,
}

impl<T> Tree<T> {
    /// Create a new empty `Tree`
    pub fn new() -> Tree<T> {
        Tree {
            nodes: Vec::new(),
            orphaned_nodes: VecDeque::new(),
        }
    }

    /// Create a new node from its associated data.
    pub fn new_node(&mut self, data: T) -> NodeId {
        let node = Node {
            parent: None,
            first_child: None,
            last_child: None,
            previous_sibling: None,
            next_sibling: None,
            data: data,
        };

        if let Some(vacant_index) = self.orphaned_nodes.pop_back() {
            self.nodes[vacant_index.index] = node;
            vacant_index
        } else {
            let next_index = self.nodes.len();
            self.nodes.push(node);
            NodeId { index: next_index }
        }
    }

    // Count nodes in arena.
    pub fn count(&self) -> usize {
        self.nodes.len()
    }

    // Returns true if arena has no nodes, false otherwise
    pub fn is_empty(&self) -> bool {
        self.count() == 0
    }

    /// Get a reference to the node with the given id if in the arena, None otherwise.
    pub fn get(&self, id: NodeId) -> Option<&Node<T>> {
        self.nodes.get(id.index)
    }

    /// Get a mutable reference to the node with the given id if in the arena, None otherwise.
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node<T>> {
        self.nodes.get_mut(id.index)
    }

    /// Iterate over all nodes in the arena in storage-order.
    pub fn iter(&self) -> std::slice::Iter<Node<T>> {
        self.nodes.iter()
    }

    pub fn clear(&mut self) {
        self.nodes.clear();
        self.orphaned_nodes.clear();
    }

    fn orphan_node(&mut self, id: NodeId) {
        let mut children = id.traverse(); // TODO: Use Traverse walker
        while let Some(child_edge) = children.walk_next(self) {
            match child_edge {
                NodeEdge::Start(node_id) |
                NodeEdge::End(node_id) => {
                    self.orphaned_nodes.push_back(id);
                }
            }
        }
    }
}

impl<T: Clone> Tree<T> {
    // TODO: Move this methods to NodeId or move NodeId methods to Tree.
    fn append_subtree_from(&mut self, from_id: NodeId, to_id: NodeId, from: &Tree<T>) {
        let new_root_id = self.new_node(from[from_id].data.clone());
        to_id.append(new_root_id, self);

        // TODO: Use the Traverse walker here.
        for child_id in from_id.children(from).iter(from) {
            let new_node = self.new_node(from[child_id].data.clone());
            new_root_id.append(new_node, self);
            self.append_subtree_from(child_id, new_node, from);
        }
    }

    pub fn extract_subtree(&self, id: NodeId) -> Tree<T> {
        let mut new_tree = Tree::new();
        self.extract_subtree_into(id, &mut new_tree);
        new_tree
    }

    pub fn extract_subtree_into(&self, id: NodeId, tree: &mut Tree<T>) {
        let new_root_id = tree.new_node(self[id].data.clone());
        tree.append_subtree_from(id, new_root_id, self);
    }
}

#[cfg(feature = "par_iter")]
impl<T: Sync> Tree<T> {
    /// Return an parallel iterator over the whole arena.
    pub fn par_iter(&self) -> rayon::slice::Iter<Node<T>> {
        self.nodes.par_iter()
    }
}

trait GetPairMut<T> {
    /// Get mutable references to two distinct nodes. Panics if the two given IDs are the same.
    fn get_pair_mut(&mut self, a: usize, b: usize, same_index_error_message: &'static str) -> (&mut T, &mut T);
}

impl<T> GetPairMut<T> for Vec<T> {
    fn get_pair_mut(&mut self, a: usize, b: usize, same_index_error_message: &'static str) -> (&mut T, &mut T) {
        if a == b {
            panic!(same_index_error_message)
        }
        let (xs, ys) = self.split_at_mut(std::cmp::max(a, b));
        if a < b {
            (&mut xs[a], &mut ys[0])
        } else {
            (&mut ys[0], &mut xs[b])
        }
    }
}

impl<T> Index<NodeId> for Tree<T> {
    type Output = Node<T>;

    fn index(&self, node: NodeId) -> &Node<T> {
        &self.nodes[node.index]
    }
}

impl<T> IndexMut<NodeId> for Tree<T> {
    fn index_mut(&mut self, node: NodeId) -> &mut Node<T> {
        &mut self.nodes[node.index]
    }
}

impl<T> Node<T> {
    /// Return the ID of the parent node, unless this node is the root of the tree.
    pub fn parent(&self) -> Option<NodeId> {
        self.parent
    }

    /// Return the ID of the first child of this node, unless it has no child.
    pub fn first_child(&self) -> Option<NodeId> {
        self.first_child
    }

    /// Return the ID of the last child of this node, unless it has no child.
    pub fn last_child(&self) -> Option<NodeId> {
        self.last_child
    }

    /// Return the ID of the previous sibling of this node, unless it is a first child.
    pub fn previous_sibling(&self) -> Option<NodeId> {
        self.previous_sibling
    }

    /// Return the ID of the previous sibling of this node, unless it is a first child.
    pub fn next_sibling(&self) -> Option<NodeId> {
        self.next_sibling
    }
}

impl NodeId {
    /// Create a `NodeId` used for attempting to get `Node`s references from an `Tree`.
    pub fn new(index: usize) -> Self {
        Self { index }
    }

    /// Return an iterator of references to this node and its ancestors.
    ///
    /// Call `.next().unwrap()` once on the iterator to skip the node itself.
    pub fn ancestors(self) -> Ancestors {
        Ancestors {
            node: Some(self),
        }
    }

    /// Return an iterator of references to this node and the siblings before it.
    ///
    /// Call `.next().unwrap()` once on the iterator to skip the node itself.
    pub fn preceding_siblings(self) -> PrecedingSiblings {
        PrecedingSiblings {
            node: Some(self),
        }
    }

    /// Return an iterator of references to this node and the siblings after it.
    ///
    /// Call `.next().unwrap()` once on the iterator to skip the node itself.
    pub fn following_siblings(self) -> FollowingSiblings {
        FollowingSiblings {
            node: Some(self),
        }
    }

    /// Return an iterator of references to this node’s children.
    pub fn children<T>(self, arena: &Tree<T>) -> Children {
        Children {
            node: arena[self].first_child,
        }
    }

    /// Return an iterator of references to this node’s children, in reverse order.
    pub fn reverse_children<T>(self, arena: &Tree<T>) -> ReverseChildren {
        ReverseChildren {
            node: arena[self].last_child,
        }
    }

    /// Return an iterator of references to this node and its descendants, in tree order.
    ///
    /// Parent nodes appear before the descendants.
    /// Call `.next().unwrap()` once on the iterator to skip the node itself.
    pub fn descendants(self) -> Descendants {
        Descendants(self.traverse())
    }

    /// Return an iterator of references to this node and its descendants, in tree order.
    pub fn traverse(self) -> Traverse {
        Traverse {
            root: self,
            next: Some(NodeEdge::Start(self)),
        }
    }

    /// Return an iterator of references to this node and its descendants, in tree order.
    pub fn reverse_traverse(self) -> ReverseTraverse {
        ReverseTraverse {
            root: self,
            next: Some(NodeEdge::End(self)),
        }
    }

    /// Detach a node from its parent and siblings. Children are not affected.
    pub fn detach<T>(self, arena: &mut Tree<T>) {
        let (parent, previous_sibling, next_sibling) = {
            let node = &mut arena[self];
            (
                node.parent.take(),
                node.previous_sibling.take(),
                node.next_sibling.take(),
            )
        };

        if let Some(next_sibling) = next_sibling {
            arena[next_sibling].previous_sibling = previous_sibling;
        } else if let Some(parent) = parent {
            arena[parent].last_child = previous_sibling;
        }

        if let Some(previous_sibling) = previous_sibling {
            arena[previous_sibling].next_sibling = next_sibling;
        } else if let Some(parent) = parent {
            arena[parent].first_child = next_sibling;
        }
    }

    /// Append a new child to this node, after existing children.
    pub fn append<T>(self, new_child: NodeId, arena: &mut Tree<T>) {
        new_child.detach(arena);
        let last_child_opt;
        {
            let (self_borrow, new_child_borrow) =
                arena
                    .nodes
                    .get_pair_mut(self.index, new_child.index, "Can not append a node to itself");
            new_child_borrow.parent = Some(self);
            last_child_opt = mem::replace(&mut self_borrow.last_child, Some(new_child));
            if let Some(last_child) = last_child_opt {
                new_child_borrow.previous_sibling = Some(last_child);
            } else {
                debug_assert!(self_borrow.first_child.is_none());
                self_borrow.first_child = Some(new_child);
            }
        }
        if let Some(last_child) = last_child_opt {
            debug_assert!(arena[last_child].next_sibling.is_none());
            arena[last_child].next_sibling = Some(new_child);
        }
    }

    /// Prepend a new child to this node, before existing children.
    pub fn prepend<T>(self, new_child: NodeId, arena: &mut Tree<T>) {
        new_child.detach(arena);
        let first_child_opt;
        {
            let (self_borrow, new_child_borrow) =
                arena
                    .nodes
                    .get_pair_mut(self.index, new_child.index, "Can not prepend a node to itself");
            new_child_borrow.parent = Some(self);
            first_child_opt = mem::replace(&mut self_borrow.first_child, Some(new_child));
            if let Some(first_child) = first_child_opt {
                new_child_borrow.next_sibling = Some(first_child);
            } else {
                self_borrow.last_child = Some(new_child);
                debug_assert!(&self_borrow.first_child.is_none());
            }
        }
        if let Some(first_child) = first_child_opt {
            debug_assert!(arena[first_child].previous_sibling.is_none());
            arena[first_child].previous_sibling = Some(new_child);
        }
    }

    /// Copies and appends the root node with its descendants to this node.
    pub fn append_subtree<T: Clone>(&mut self, from_tree: &Tree<T>, into_tree: &mut Tree<T>) {
        let from_root_id = NodeId::new(0);
        if let Some(_) = from_tree.get(from_root_id) {
            into_tree.append_subtree_from(from_root_id, *self, from_tree);
        }
    }

    /// Detaches and marks the node and its children as reusable.
    pub fn orphan<T>(self, arena: &mut Tree<T>) {
        self.detach(arena);
        arena.orphan_node(self);
    }

    /// Insert a new sibling after this node.
    pub fn insert_after<T>(self, new_sibling: NodeId, arena: &mut Tree<T>) {
        new_sibling.detach(arena);
        let next_sibling_opt;
        let parent_opt;
        {
            let (self_borrow, new_sibling_borrow) =
                arena
                    .nodes
                    .get_pair_mut(self.index, new_sibling.index, "Can not insert a node after itself");
            parent_opt = self_borrow.parent;
            new_sibling_borrow.parent = parent_opt;
            new_sibling_borrow.previous_sibling = Some(self);
            next_sibling_opt = mem::replace(&mut self_borrow.next_sibling, Some(new_sibling));
            if let Some(next_sibling) = next_sibling_opt {
                new_sibling_borrow.next_sibling = Some(next_sibling);
            }
        }
        if let Some(next_sibling) = next_sibling_opt {
            debug_assert!(arena[next_sibling].previous_sibling.unwrap() == self);
            arena[next_sibling].previous_sibling = Some(new_sibling);
        } else if let Some(parent) = parent_opt {
            debug_assert!(arena[parent].last_child.unwrap() == self);
            arena[parent].last_child = Some(new_sibling);
        }
    }

    /// Insert a new sibling before this node.
    pub fn insert_before<T>(self, new_sibling: NodeId, arena: &mut Tree<T>) {
        new_sibling.detach(arena);
        let previous_sibling_opt;
        let parent_opt;
        {
            let (self_borrow, new_sibling_borrow) =
                arena
                    .nodes
                    .get_pair_mut(self.index, new_sibling.index, "Can not insert a node before itself");
            parent_opt = self_borrow.parent;
            new_sibling_borrow.parent = parent_opt;
            new_sibling_borrow.next_sibling = Some(self);
            previous_sibling_opt = mem::replace(&mut self_borrow.previous_sibling, Some(new_sibling));
            if let Some(previous_sibling) = previous_sibling_opt {
                new_sibling_borrow.previous_sibling = Some(previous_sibling);
            }
        }
        if let Some(previous_sibling) = previous_sibling_opt {
            debug_assert!(arena[previous_sibling].next_sibling.unwrap() == self);
            arena[previous_sibling].next_sibling = Some(new_sibling);
        } else if let Some(parent) = parent_opt {
            debug_assert!(arena[parent].first_child.unwrap() == self);
            arena[parent].first_child = Some(new_sibling);
        }
    }
}

macro_rules! impl_node_walker {
    ($name:ident, $next:expr) => {
        impl<T> Walker<T> for $name {
            type Item = NodeId;

            fn walk_next(&mut self, arena: &Tree<T>) -> Option<NodeId> {
                match self.node.take() {
                    Some(node) => {
                        self.node = $next(&arena[node]);
                        Some(node)
                    }
                    None => None,
                }
            }
        }
    };
}

/// An iterator of references to the ancestors a given node.
pub struct Ancestors {
    node: Option<NodeId>,
}
impl_node_walker!(Ancestors, |node: &Node<T>| node.parent);

/// An iterator of references to the siblings before a given node.
pub struct PrecedingSiblings {
    node: Option<NodeId>,
}
impl_node_walker!(PrecedingSiblings, |node: &Node<T>| node.previous_sibling);

/// An iterator of references to the siblings after a given node.
pub struct FollowingSiblings {
    node: Option<NodeId>,
}
impl_node_walker!(FollowingSiblings, |node: &Node<T>| node.next_sibling);

/// An iterator of references to the children of a given node.
pub struct Children {
    node: Option<NodeId>,
}
impl_node_walker!(Children, |node: &Node<T>| node.next_sibling);

/// An iterator of references to the children of a given node, in reverse order.
pub struct ReverseChildren {
    node: Option<NodeId>,
}
impl_node_walker!(ReverseChildren, |node: &Node<T>| node.previous_sibling);

/// An iterator of references to a given node and its descendants, in tree order.
pub struct Descendants(Traverse);

impl<T> Walker<T> for Descendants {
    type Item = NodeId;

    fn walk_next(&mut self, arena: &Tree<T>) -> Option<NodeId> {
        loop {
            match self.0.walk_next(arena) {
                Some(NodeEdge::Start(node)) => return Some(node),
                Some(NodeEdge::End(_)) => {}
                None => return None,
            }
        }
    }
}

#[derive(Debug, Clone)]
/// Indicator if the node is at a start or endpoint of the tree
pub enum NodeEdge<T> {
    /// Indicates that start of a node that has children. Yielded by `Traverse::next` before the
    /// node’s descendants. In HTML or XML, this corresponds to an opening tag like `<div>`
    Start(T),

    /// Indicates that end of a node that has children. Yielded by `Traverse::next` after the
    /// node’s descendants. In HTML or XML, this corresponds to a closing tag like `</div>`
    End(T),
}

/// An iterator of references to a given node and its descendants, in tree order.
pub struct Traverse {
    root: NodeId,
    next: Option<NodeEdge<NodeId>>,
}

impl<T> Walker<T> for Traverse {
    type Item = NodeEdge<NodeId>;

    fn walk_next(&mut self, arena: &Tree<T>) -> Option<NodeEdge<NodeId>> {
        match self.next.take() {
            Some(item) => {
                self.next = match item {
                    NodeEdge::Start(node) => match arena[node].first_child {
                        Some(first_child) => Some(NodeEdge::Start(first_child)),
                        None => Some(NodeEdge::End(node)),
                    },
                    NodeEdge::End(node) => {
                        if node == self.root {
                            None
                        } else {
                            match arena[node].next_sibling {
                                Some(next_sibling) => Some(NodeEdge::Start(next_sibling)),
                                None => {
                                    match arena[node].parent {
                                        Some(parent) => Some(NodeEdge::End(parent)),

                                        // `node.parent()` here can only be `None`
                                        // if the tree has been modified during iteration,
                                        // but silently stoping iteration
                                        // seems a more sensible behavior than panicking.
                                        None => None,
                                    }
                                }
                            }
                        }
                    }
                };
                Some(item)
            }
            None => None,
        }
    }
}

/// An iterator of references to a given node and its descendants, in reverse tree order.
pub struct ReverseTraverse {
    root: NodeId,
    next: Option<NodeEdge<NodeId>>,
}

impl<T> Walker<T> for ReverseTraverse {
    type Item = NodeEdge<NodeId>;

    fn walk_next(&mut self, arena: &Tree<T>) -> Option<NodeEdge<NodeId>> {
        match self.next.take() {
            Some(item) => {
                self.next = match item {
                    NodeEdge::End(node) => match arena[node].last_child {
                        Some(last_child) => Some(NodeEdge::End(last_child)),
                        None => Some(NodeEdge::Start(node)),
                    },
                    NodeEdge::Start(node) => {
                        if node == self.root {
                            None
                        } else {
                            match arena[node].previous_sibling {
                                Some(previous_sibling) => Some(NodeEdge::End(previous_sibling)),
                                None => {
                                    match arena[node].parent {
                                        Some(parent) => Some(NodeEdge::Start(parent)),

                                        // `node.parent()` here can only be `None`
                                        // if the tree has been modified during iteration,
                                        // but silently stoping iteration
                                        // seems a more sensible behavior than panicking.
                                        None => None,
                                    }
                                }
                            }
                        }
                    }
                };
                Some(item)
            }
            None => None,
        }
    }
}
