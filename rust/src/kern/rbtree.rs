// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>

//! Red-black tree, which `kern/rbtree.c` used to define.
//!
//! The tree is intrusive: `RbtreeNode` is layout-identical to `struct
//! rbtree_node` of <kern/rbtree_i.h> and lives inside the caller's
//! structures.  The generic lookups and inserts stay macros in
//! <kern/rbtree.h>, where the comparison function is known at the call
//! site; the functions here are the non-generic half those macros call.
//! They keep the C signatures exactly, so `vm/vm_map.c` and `kern/slab.c`
//! keep using the same headers.
//!
//! The parent member packs the color in its low bit
//! (`rbtree_i.h:38-47,60-74`), so `RbtreeNode` must be 4-byte aligned;
//! the layout constants below fail the build if the mirror drifts.
//!
//! The core is Rust-native: a copyable `NodeRef` handle and methods on
//! `Rbtree` carry the algorithms, and the nine `extern "C"` functions
//! are thin adapters for `vm/vm_map.c` and `kern/slab.c`.  `NodeRef`
//! moves by value and never borrows node storage, because the tree's
//! links alias the same nodes; the raw pointers appear only at the
//! adapters and a few helpers.
//!
//! This module must stay free of `crate::` imports: `tests/test-rbtree-rs`
//! compiles it for the host with `rustc --test`.

use core::ffi::c_int;
use core::mem::{align_of, offset_of, size_of};
use core::ptr::{self, NonNull};

/// Child indexes, `RBTREE_LEFT`/`RBTREE_RIGHT` of <kern/rbtree.h>.
pub const RBTREE_LEFT: c_int = 0;
pub const RBTREE_RIGHT: c_int = 1;

/// The private helpers index the same array.
const LEFT: usize = RBTREE_LEFT as usize;
const RIGHT: usize = RBTREE_RIGHT as usize;

/// The parent masks of <kern/rbtree_i.h>.
const COLOR_MASK: usize = 0x1;
const PARENT_MASK: usize = !0x3;

/// The two colors of a red-black node.  The value never leaves this
/// module: C only ever sees the low bit of `parent`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Color {
    Red,
    Black,
}

impl Color {
    /// The low bit stored in `parent`.
    const fn bit(self) -> usize {
        match self {
            Color::Red => 0,
            Color::Black => 1,
        }
    }

    /// The color a stored low bit stands for.
    const fn from_bit(bit: usize) -> Color {
        if bit == 0 { Color::Red } else { Color::Black }
    }
}

/// A child side.  The C boundary passes it as `c_int` 0/1; internally
/// the typed side keeps the rotations and walks free of index math.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Left,
    Right,
}

impl Side {
    /// The children array index.
    const fn index(self) -> usize {
        match self {
            Side::Left => LEFT,
            Side::Right => RIGHT,
        }
    }

    /// The other side.
    const fn opposite(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }

    /// The side a C direction/index argument stands for; anything but
    /// `RBTREE_LEFT` counts as right, like `rbtree_d2i()`.
    const fn from_int(direction: c_int) -> Side {
        if direction == RBTREE_LEFT {
            Side::Left
        } else {
            Side::Right
        }
    }
}

/// `RBTREE_SLOT_*` of <kern/rbtree_i.h>: how `rbtree_slot()` packs an
/// insertion point.
const SLOT_INDEX_MASK: usize = 0x1;
const SLOT_PARENT_MASK: usize = !SLOT_INDEX_MASK;

/// `struct rbtree_node`: a parent address with the color in its low
/// bit, and the two children.
#[repr(C)]
pub struct RbtreeNode {
    parent: usize,
    children: [Option<NonNull<RbtreeNode>>; 2],
}

/// `struct rbtree`.
#[repr(C)]
pub struct Rbtree {
    root: Option<NonNull<RbtreeNode>>,
}

// The C header defines both structures and embeds them; these asserts
// pin the mirror to the same layout.
const _: () = assert!(size_of::<RbtreeNode>() == 3 * size_of::<usize>());
const _: () = assert!(align_of::<RbtreeNode>() == align_of::<usize>());
const _: () = assert!(align_of::<RbtreeNode>() >= 4);
const _: () = assert!(offset_of!(RbtreeNode, children) == size_of::<usize>());
const _: () = assert!(size_of::<Rbtree>() == size_of::<*mut RbtreeNode>());
const _: () = assert!(align_of::<Rbtree>() == align_of::<*mut RbtreeNode>());

impl RbtreeNode {
    /// Node storage with null links, before `init()` or
    /// `rbtree_node_init()` makes it an unlinked tree node.
    const fn unlinked() -> Self {
        Self {
            parent: 0,
            children: [None, None],
        }
    }

    /// Make this node unlinked: parent itself, red, null children.
    /// `rbtree_node_init()` uses this.
    fn init(&mut self) {
        *self = Self::unlinked();
        self.parent = ptr::from_ref(self).addr() | Color::Red.bit();
    }
}

impl Rbtree {
    /// An empty tree; `rbtree_init()` uses this.
    const fn new() -> Self {
        Self { root: None }
    }

    /// Reset the tree to empty.  `rbtree_init()` in C.
    pub(crate) fn init(&mut self) {
        self.root = None;
    }

    /// Walk to the node whose comparison is zero, or the nearest in
    /// `direction` when none is.  The C `rbtree_lookup_nearest()`
    /// macro, with the comparison moved into `cmp`: it receives each
    /// visited node and returns the ordering of the key against it.
    pub(crate) fn lookup_nearest<F>(
        &self,
        cmp: F,
        direction: c_int,
    ) -> Option<NonNull<RbtreeNode>>
    where
        F: Fn(NonNull<RbtreeNode>) -> c_int,
    {
        let mut prev = self.root;
        let mut index = -1;
        let mut cur = self.root;

        while let Some(node) = cur {
            let diff = cmp(node);
            if diff == 0 {
                return Some(node);
            }
            prev = cur;
            index = rbtree_d2i(diff);
            // SAFETY: the caller promises a valid tree, so a visited
            // node's children are valid nodes or null.
            cur = unsafe {
                (*node.as_ptr()).children[Side::from_int(index).index()]
            };
        }

        // SAFETY: `prev` is null or the last valid node visited.
        let found = unsafe { nearest(prev.map(NodeRef), index, direction) };
        found.map(|node| node.0)
    }

    /// The root, if any.
    fn root(&self) -> Option<NodeRef> {
        self.root.map(NodeRef)
    }

    /// Change the root.
    fn set_root(&mut self, node: Option<NodeRef>) {
        self.root = node.map(|n| n.0);
    }

    /// Link `node` at the point `parent`/`side` names, rebalancing.
    ///
    /// # Safety
    ///
    /// `node` must be unlinked caller storage, and `parent`/`side` must
    /// be an insertion point in this tree.
    unsafe fn insert(
        &mut self,
        parent: Option<NodeRef>,
        side: Side,
        node: NodeRef,
    ) {
        // SAFETY: the caller promises a valid, unlinked node.
        unsafe {
            node.set_parent(parent);
            node.set_color(Color::Red);
            node.set_child(Side::Left, None);
            node.set_child(Side::Right, None);

            match parent {
                None => self.set_root(Some(node)),
                Some(p) => p.set_child(side, Some(node)),
            }
        }

        let mut node = node;
        let mut parent = parent;

        loop {
            let Some(p) = parent else {
                // SAFETY: the caller promises a valid node.
                unsafe { node.set_color(Color::Black) };
                break;
            };

            // SAFETY: the caller promises valid nodes.
            if unsafe { p.is_black() } {
                break;
            }

            // A red node always has a parent in a valid tree.
            // SAFETY: as above.
            let grand_parent = unsafe { p.parent() }
                .expect("rbtree: red node without a grandparent");
            // SAFETY: as above.
            let side = unsafe { grand_parent.child_index(Some(p)) };
            let other = side.opposite();
            // SAFETY: as above.
            let uncle = unsafe { grand_parent.child(other) };

            // Uncle is red: flip colors and repeat at the grandparent.
            if let Some(uncle) = uncle {
                // SAFETY: as above.
                if unsafe { uncle.is_red() } {
                    // SAFETY: as above.
                    unsafe {
                        uncle.set_color(Color::Black);
                        p.set_color(Color::Black);
                        grand_parent.set_color(Color::Red);
                    }
                    node = grand_parent;
                    // SAFETY: as above.
                    parent = unsafe { node.parent() };
                    continue;
                }
            }

            // Node is the opposite child of its parent: rotate at the
            // parent and blacken the node, which takes its place.
            // SAFETY: as above.
            let final_parent = if unsafe { p.child(other) } == Some(node) {
                // SAFETY: as above.
                unsafe { self.rotate(p, side) };
                node
            } else {
                p
            };

            // Node is the near child: recolor, rotate at the
            // grandparent, and leave.
            // SAFETY: as above.
            unsafe {
                final_parent.set_color(Color::Black);
                grand_parent.set_color(Color::Red);
                self.rotate(grand_parent, other);
            }
            break;
        }
    }

    /// Remove `node`, then restore the red-black rules.
    ///
    /// After completion the node is stale: its links are not cleared,
    /// as the C comment in <kern/rbtree.h> says.
    ///
    /// # Safety
    ///
    /// `node` must be linked in this tree.
    unsafe fn remove(&mut self, node: NodeRef) {
        let mut child: Option<NodeRef>;
        let mut parent: Option<NodeRef>;
        let color: Color;

        // SAFETY: the caller promises a valid node.
        let (left, right) =
            unsafe { (node.child(Side::Left), node.child(Side::Right)) };

        match (left, right) {
            // Node has at most one child.
            (None, right) => {
                child = right;
                // SAFETY: as above.
                color = unsafe { node.color() };
                // SAFETY: as above.
                parent = unsafe { node.parent() };

                // SAFETY: as above.
                unsafe {
                    if let Some(c) = child {
                        c.set_parent(parent);
                    }

                    match parent {
                        None => self.set_root(child),
                        Some(p) => {
                            p.set_child(p.child_index(Some(node)), child)
                        }
                    }
                }
            }
            (left, None) => {
                child = left;
                // SAFETY: as above.
                color = unsafe { node.color() };
                // SAFETY: as above.
                parent = unsafe { node.parent() };

                // SAFETY: as above.
                unsafe {
                    if let Some(c) = child {
                        c.set_parent(parent);
                    }

                    match parent {
                        None => self.set_root(child),
                        Some(p) => {
                            p.set_child(p.child_index(Some(node)), child)
                        }
                    }
                }
            }
            // Two children: replace the node with its successor.
            (Some(_), Some(right)) => {
                let mut successor = right;
                let mut successor_parent = node;
                // SAFETY: as above.
                while let Some(l) = unsafe { successor.child(Side::Left) } {
                    successor_parent = successor;
                    successor = l;
                }

                // SAFETY: as above.
                color = unsafe { successor.color() };
                child = unsafe { successor.child(Side::Right) };
                parent = unsafe { node.parent() };

                // SAFETY: as above.
                unsafe {
                    match parent {
                        None => self.set_root(Some(successor)),
                        Some(p) => p.set_child(
                            p.child_index(Some(node)),
                            Some(successor),
                        ),
                    }

                    // Copy the whole parent word to keep the original
                    // color.
                    successor.copy_parent_from(node);
                    successor.set_child(Side::Left, node.child(Side::Left));
                    if let Some(l) = successor.child(Side::Left) {
                        l.set_parent(Some(successor));
                    }
                }

                if successor_parent == node {
                    parent = Some(successor);
                } else {
                    // SAFETY: as above.
                    unsafe {
                        successor
                            .set_child(Side::Right, node.child(Side::Right));
                        if let Some(r) = successor.child(Side::Right) {
                            r.set_parent(Some(successor));
                        }

                        successor_parent.set_child(Side::Left, child);
                        if let Some(c) = child {
                            c.set_parent(Some(successor_parent));
                        }
                    }
                    parent = Some(successor_parent);
                }
            }
        }

        // Update the colors; a null child counts as a black leaf.
        if color == Color::Red {
            return;
        }

        loop {
            if let Some(c) = child {
                // SAFETY: the caller promises valid links.
                if unsafe { c.is_red() } {
                    // SAFETY: as above.
                    unsafe { c.set_color(Color::Black) };
                    break;
                }
            }

            let Some(p) = parent else {
                break;
            };

            // SAFETY: as above.
            let side = unsafe { p.child_index(child) };
            let other = side.opposite();
            // SAFETY: as above.
            let mut brother = unsafe { p.child(other) }
                .expect("rbtree: black node without a brother");

            // Brother is red: recolor and rotate at the parent so that
            // the brother becomes black.
            // SAFETY: as above.
            if unsafe { brother.is_red() } {
                // SAFETY: as above.
                unsafe {
                    brother.set_color(Color::Black);
                    p.set_color(Color::Red);
                    self.rotate(p, side);
                }
                // SAFETY: as above.
                brother = unsafe { p.child(other) }
                    .expect("rbtree: black node without a brother");
            }

            // SAFETY: as above.
            let brother_side = unsafe { brother.child(side) };
            // SAFETY: as above.
            let brother_other = unsafe { brother.child(other) };

            // Brother has no red child: recolor and repeat at the
            // parent.
            // SAFETY: as above.
            let side_black =
                brother_side.is_none_or(|n| unsafe { n.is_black() });
            // SAFETY: as above.
            let other_black =
                brother_other.is_none_or(|n| unsafe { n.is_black() });
            if side_black && other_black {
                // SAFETY: as above.
                unsafe { brother.set_color(Color::Red) };
                child = Some(p);
                // SAFETY: as above.
                parent = unsafe { p.parent() };
                continue;
            }

            // Brother's far child is black: recolor and rotate at the
            // brother.
            if other_black {
                let brother_side = brother_side
                    .expect("rbtree: black node without a red child");
                // SAFETY: as above.
                unsafe {
                    brother_side.set_color(Color::Black);
                    brother.set_color(Color::Red);
                    self.rotate(brother, other);
                }
                // SAFETY: as above.
                brother = unsafe { p.child(other) }
                    .expect("rbtree: black node without a brother");
            }

            // Exchange the parent and brother colors, blacken the
            // brother's far child, rotate at the parent, and leave.
            // SAFETY: as above.
            unsafe {
                brother.set_color(p.color());
                p.set_color(Color::Black);
                let brother_other = brother
                    .child(other)
                    .expect("rbtree: brother without a far child");
                brother_other.set_color(Color::Black);
                self.rotate(p, side);
            }
            break;
        }
    }

    /// The first node toward `side`, if the tree is not empty.
    ///
    /// # Safety
    ///
    /// This tree must be valid.
    unsafe fn firstlast(&self, side: Side) -> Option<NodeRef> {
        let mut prev = None;
        let mut cur = self.root();

        while let Some(node) = cur {
            prev = Some(node);
            // SAFETY: the caller promises a valid tree.
            cur = unsafe { node.child(side) };
        }

        prev
    }

    /// Rotate the subtree rooted at `node` toward `side`.
    ///
    /// # Safety
    ///
    /// `node` must be linked in this tree, and its child on the
    /// opposite side must exist.
    unsafe fn rotate(&mut self, node: NodeRef, side: Side) {
        let other = side.opposite();
        // SAFETY: the caller promises a linked node.
        let parent = unsafe { node.parent() };
        // SAFETY: as above.
        let rnode = unsafe { node.child(other) }
            .expect("rbtree: rotate at a node without its child");

        // SAFETY: the caller promises all of these nodes are valid.
        unsafe {
            node.set_child(other, rnode.child(side));

            if let Some(c) = rnode.child(side) {
                c.set_parent(Some(node));
            }

            rnode.set_child(side, Some(node));
            rnode.set_parent(parent);

            match parent {
                None => self.set_root(Some(rnode)),
                Some(p) => {
                    p.set_child(p.child_index(Some(node)), Some(rnode));
                }
            }

            node.set_parent(Some(rnode));
        }
    }

    /// The node next to, or previous to, `node`.  Private: C reaches
    /// `rbtree_nearest()` instead.
    ///
    /// # Safety
    ///
    /// `node` must be a valid, linked node.
    unsafe fn walk(node: Option<NodeRef>, side: Side) -> Option<NodeRef> {
        let other = side.opposite();
        let mut node = node?;

        // SAFETY: the caller promises a valid node.
        if let Some(mut child) = unsafe { node.child(side) } {
            loop {
                // SAFETY: as above.
                match unsafe { child.child(other) } {
                    Some(next) => child = next,
                    None => {
                        node = child;
                        break;
                    }
                }
            }
        } else {
            loop {
                // SAFETY: as above.
                let parent = (unsafe { node.parent() })?;
                // SAFETY: as above.
                let index = unsafe { parent.child_index(Some(node)) };
                node = parent;

                if index == other {
                    break;
                }
            }
        }

        Some(node)
    }
}

/// A non-null, valid tree node.
///
/// Copyable by design: the tree's links alias nodes, so the core moves
/// these handles by value and never creates a Rust reference to node
/// storage.  Every dereference lives in these methods, each with its
/// own safety argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NodeRef(NonNull<RbtreeNode>);

impl NodeRef {
    /// View a raw node the caller promises is non-null.
    ///
    /// # Safety
    ///
    /// `node` must not be null.
    unsafe fn new(node: *mut RbtreeNode) -> NodeRef {
        // SAFETY: the caller promises non-null.
        NodeRef(unsafe { NonNull::new_unchecked(node) })
    }

    /// The raw pointer, for the C adapters and the tests.
    fn as_ptr(self) -> *mut RbtreeNode {
        self.0.as_ptr()
    }

    /// The parent address without the color bit.
    ///
    /// # Safety
    ///
    /// The node must be valid.
    unsafe fn parent(self) -> Option<NodeRef> {
        // SAFETY: the caller promises a valid node.
        let addr = unsafe { (*self.as_ptr()).parent } & PARENT_MASK;
        NonNull::new(ptr::with_exposed_provenance_mut(addr)).map(NodeRef)
    }

    /// Set the parent, retaining the color.
    ///
    /// # Safety
    ///
    /// The node must be valid.
    unsafe fn set_parent(self, parent: Option<NodeRef>) {
        // SAFETY: the caller promises a valid node.
        unsafe {
            let addr = parent.map_or(0, |p| p.as_ptr().addr());
            let node = self.as_ptr();
            (*node).parent = addr | ((*node).parent & COLOR_MASK);
        }
    }

    /// Copy another node's parent word, color included.
    ///
    /// # Safety
    ///
    /// Both nodes must be valid.
    unsafe fn copy_parent_from(self, other: NodeRef) {
        // SAFETY: the caller promises valid nodes.
        unsafe { (*self.as_ptr()).parent = (*other.as_ptr()).parent };
    }

    /// The node's color.
    ///
    /// # Safety
    ///
    /// The node must be valid.
    unsafe fn color(self) -> Color {
        // SAFETY: the caller promises a valid node.
        unsafe { Color::from_bit((*self.as_ptr()).parent & COLOR_MASK) }
    }

    /// Set the color, retaining the parent.
    ///
    /// # Safety
    ///
    /// The node must be valid.
    unsafe fn set_color(self, color: Color) {
        // SAFETY: the caller promises a valid node.
        unsafe {
            let node = self.as_ptr();
            (*node).parent = ((*node).parent & PARENT_MASK) | color.bit();
        }
    }

    /// Whether the node is red.
    ///
    /// # Safety
    ///
    /// The node must be valid.
    unsafe fn is_red(self) -> bool {
        // SAFETY: the caller promises a valid node.
        unsafe { self.color() == Color::Red }
    }

    /// Whether the node is black.
    ///
    /// # Safety
    ///
    /// The node must be valid.
    unsafe fn is_black(self) -> bool {
        // SAFETY: the caller promises a valid node.
        unsafe { self.color() == Color::Black }
    }

    /// The child on `side`, if any.
    ///
    /// # Safety
    ///
    /// The node must be valid.
    unsafe fn child(self, side: Side) -> Option<NodeRef> {
        // SAFETY: the caller promises a valid node.
        unsafe { (*self.as_ptr()).children[side.index()] }.map(NodeRef)
    }

    /// Change the child on `side`.
    ///
    /// # Safety
    ///
    /// The node must be valid, and `child` valid or null.
    unsafe fn set_child(self, side: Side, child: Option<NodeRef>) {
        // SAFETY: the caller promises valid nodes.
        unsafe {
            (*self.as_ptr()).children[side.index()] = child.map(|n| n.0);
        }
    }

    /// The side `child` is on.  A null child counts as the left one
    /// when the left link is null; `rbtree_index()` in C works the same
    /// way, and the remove path depends on it.
    ///
    /// # Safety
    ///
    /// The node must be valid, and `child` one of its children (or
    /// null where one child is null).
    unsafe fn child_index(self, child: Option<NodeRef>) -> Side {
        // SAFETY: the caller promises a valid node.
        if unsafe { (*self.as_ptr()).children[LEFT] } == child.map(|n| n.0) {
            Side::Left
        } else {
            Side::Right
        }
    }
}

/// The nearest node to a failed lookup, given the last node visited,
/// the side taken there, and the direction wanted.
///
/// # Safety
///
/// `parent` must be a valid, linked node.
unsafe fn nearest(
    parent: Option<NodeRef>,
    index: c_int,
    direction: c_int,
) -> Option<NodeRef> {
    let parent = parent?;

    if index != direction {
        Some(parent)
    } else {
        // SAFETY: the caller promises a valid node.
        unsafe { Rbtree::walk(Some(parent), Side::from_int(direction)) }
    }
}

/// Initialize a tree.  `rbtree_init()` in C.
///
/// # Safety
///
/// `tree` must point at storage for a `struct rbtree`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rbtree_init(tree: *mut Rbtree) {
    // SAFETY: the caller promises valid storage.
    unsafe { *tree = Rbtree::new() };
}

/// Initialize a node, which is in no tree while its parent is itself.
/// `rbtree_node_init()` in C.
///
/// # Safety
///
/// `node` must point at storage for a `struct rbtree_node`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rbtree_node_init(node: *mut RbtreeNode) {
    // SAFETY: the caller promises valid storage.
    unsafe { (*node).init() };
}

/// Convert a comparison result into a child index (0 or 1).
/// `rbtree_d2i()` in C.
///
/// The C lookup macros call this once per level; it is a boundary
/// function so the Rust owns the convention.
#[unsafe(no_mangle)]
pub extern "C" fn rbtree_d2i(diff: c_int) -> c_int {
    let side = if diff <= 0 { Side::Left } else { Side::Right };
    side.index() as c_int
}

/// Translate an insertion point into a slot.  `rbtree_slot()` in C.
///
/// `parent` may be null, which is the empty tree's slot 0.  `index` is
/// the child side the C macro found, 0 or 1.
#[unsafe(no_mangle)]
pub extern "C" fn rbtree_slot(parent: *mut RbtreeNode, index: c_int) -> usize {
    parent.addr() | index as usize
}

/// Insert at an insertion point.  `rbtree_insert_slot()` in C.
///
/// # Safety
///
/// `tree` must be valid, `node` must be unlinked caller storage, and
/// `slot` must come from `rbtree_slot()` on this tree.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rbtree_insert_slot(
    tree: *mut Rbtree,
    slot: usize,
    node: *mut RbtreeNode,
) {
    let parent = ptr::with_exposed_provenance_mut(slot & SLOT_PARENT_MASK);
    let index = (slot & SLOT_INDEX_MASK) as c_int;
    // SAFETY: the caller promises a valid tree, an unlinked node and a
    // slot that names a point in it.
    unsafe {
        (*tree).insert(
            NonNull::new(parent).map(NodeRef),
            Side::from_int(index),
            NodeRef::new(node),
        )
    };
}

/// Insert a node and rebalance.  `rbtree_insert_rebalance()` in C.
///
/// The caller's `rbtree_insert()` macro has already found the insertion
/// point; `index` is ignored when `parent` is null.
///
/// # Safety
///
/// `tree` and `node` must be valid, and `node` must be unlinked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rbtree_insert_rebalance(
    tree: *mut Rbtree,
    parent: *mut RbtreeNode,
    index: c_int,
    node: *mut RbtreeNode,
) {
    // SAFETY: the caller's macro passes a valid tree, the found parent
    // (or null) and caller-owned unlinked node storage.
    unsafe {
        (*tree).insert(
            NonNull::new(parent).map(NodeRef),
            Side::from_int(index),
            NodeRef::new(node),
        )
    };
}

/// Remove a node from a tree.  `rbtree_remove()` in C.
///
/// # Safety
///
/// `tree` must be valid and `node` must be linked in it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rbtree_remove(
    tree: *mut Rbtree,
    node: *mut RbtreeNode,
) {
    // SAFETY: the caller promises a valid tree and a linked node.
    unsafe { (*tree).remove(NodeRef::new(node)) };
}

/// The nearest node to a failed lookup.  `rbtree_nearest()` in C.
///
/// `parent` is the last node visited, `index` the direction taken
/// there, and `direction` either `RBTREE_LEFT` (previous) or
/// `RBTREE_RIGHT` (next).
///
/// # Safety
///
/// `parent` must be null or a valid node.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rbtree_nearest(
    parent: *mut RbtreeNode,
    index: c_int,
    direction: c_int,
) -> *mut RbtreeNode {
    // SAFETY: the caller promises a valid node or null.
    let found = unsafe {
        nearest(NonNull::new(parent).map(NodeRef), index, direction)
    };
    found.map_or(ptr::null_mut(), NodeRef::as_ptr)
}

/// The first or last node of a tree.  `rbtree_firstlast()` in C.
///
/// # Safety
///
/// `tree` must be a valid tree.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rbtree_firstlast(
    tree: *const Rbtree,
    direction: c_int,
) -> *mut RbtreeNode {
    // SAFETY: the caller promises a valid tree.
    let found = unsafe { (*tree).firstlast(Side::from_int(direction)) };
    found.map_or(ptr::null_mut(), NodeRef::as_ptr)
}

/// Host unit tests; `tests/test-rbtree-rs` compiles this file with
/// `rustc --test` and runs them.  They exercise the C macro protocols
/// (insert, lookup_slot/insert_slot, lookup_nearest) the way
/// `vm/vm_map.c` and `kern/slab.c` use them, and check the red-black
/// rules after every mutation.
#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::offset_of;

    /// A keyed entry: the node comes first, so a node pointer can be
    /// mapped back to its key.
    #[repr(C)]
    struct Entry {
        node: RbtreeNode,
        key: u32,
    }

    fn entries(keys: &[u32]) -> Vec<Entry> {
        let mut v = Vec::with_capacity(keys.len());
        for &key in keys {
            let mut node = RbtreeNode::unlinked();
            node.init();
            v.push(Entry { node, key });
        }
        v
    }

    fn new_tree() -> Rbtree {
        Rbtree::new()
    }

    fn node_of(entry: &Entry) -> *mut RbtreeNode {
        &entry.node as *const RbtreeNode as *mut RbtreeNode
    }

    /// The entry a node lives in.
    ///
    /// # Safety
    ///
    /// `node` must point at the `node` field of an `Entry`.
    unsafe fn entry_of(node: *const RbtreeNode) -> *const Entry {
        // SAFETY: the caller promises the node field's address.
        (unsafe { (node as *const u8).sub(offset_of!(Entry, node)) })
            as *const Entry
    }

    /// # Safety
    ///
    /// As `entry_of`.
    unsafe fn key_of(node: *const RbtreeNode) -> u32 {
        // SAFETY: the caller promises a node inside an Entry.
        unsafe { (*entry_of(node)).key }
    }

    /// The C `rbtree_insert()` macro.
    ///
    /// # Safety
    ///
    /// `tree` and `entry` must be valid, and the entry's key absent.
    unsafe fn insert(tree: *mut Rbtree, entry: &Entry) {
        let node = node_of(entry);
        let mut prev: *mut RbtreeNode = ptr::null_mut();
        let mut index = -1;
        // SAFETY: the caller promises a valid tree.
        let mut cur =
            unsafe { (*tree).root.map_or(ptr::null_mut(), NonNull::as_ptr) };

        while !cur.is_null() {
            // SAFETY: as above; the walk follows valid children.
            let diff = entry.key as c_int - unsafe { key_of(cur) } as c_int;
            prev = cur;
            index = rbtree_d2i(diff);
            // SAFETY: the caller promises valid links.
            cur = unsafe {
                (*cur).children[index as usize]
                    .map_or(ptr::null_mut(), NonNull::as_ptr)
            };
        }

        // SAFETY: the caller promises the entry is unlinked.
        unsafe { rbtree_insert_rebalance(tree, prev, index, node) };
    }

    /// The C `rbtree_lookup_slot()` macro: the node and an insertion
    /// point packed as `parent | index`.
    ///
    /// # Safety
    ///
    /// `tree` must be valid.
    unsafe fn lookup_slot(
        tree: *mut Rbtree,
        key: u32,
    ) -> (*mut RbtreeNode, usize) {
        let mut prev: *mut RbtreeNode = ptr::null_mut();
        let mut index = 0;
        // SAFETY: the caller promises a valid tree.
        let mut cur =
            unsafe { (*tree).root.map_or(ptr::null_mut(), NonNull::as_ptr) };

        while !cur.is_null() {
            // SAFETY: as above.
            let diff = key as c_int - unsafe { key_of(cur) } as c_int;
            if diff == 0 {
                break;
            }
            prev = cur;
            index = rbtree_d2i(diff);
            // SAFETY: the caller promises valid links.
            cur = unsafe {
                (*cur).children[index as usize]
                    .map_or(ptr::null_mut(), NonNull::as_ptr)
            };
        }

        (cur, rbtree_slot(prev, index))
    }

    /// The C `rbtree_lookup_nearest()` macro in `direction`.
    ///
    /// # Safety
    ///
    /// `tree` must be valid.
    unsafe fn lookup_nearest(
        tree: *mut Rbtree,
        key: u32,
        direction: c_int,
    ) -> *mut RbtreeNode {
        let mut prev: *mut RbtreeNode = ptr::null_mut();
        let mut index = -1;
        // SAFETY: the caller promises a valid tree.
        let mut cur =
            unsafe { (*tree).root.map_or(ptr::null_mut(), NonNull::as_ptr) };

        while !cur.is_null() {
            // SAFETY: as above.
            let diff = key as c_int - unsafe { key_of(cur) } as c_int;
            if diff == 0 {
                break;
            }
            prev = cur;
            index = rbtree_d2i(diff);
            // SAFETY: the caller promises valid links.
            cur = unsafe {
                (*cur).children[index as usize]
                    .map_or(ptr::null_mut(), NonNull::as_ptr)
            };
        }

        if cur.is_null() {
            // SAFETY: as above.
            cur = unsafe { rbtree_nearest(prev, index, direction) };
        }
        cur
    }

    /// # Safety
    ///
    /// `tree` must be valid.
    unsafe fn find(tree: *mut Rbtree, key: u32) -> *mut RbtreeNode {
        // SAFETY: the caller promises a valid tree.
        let mut cur =
            unsafe { (*tree).root.map_or(ptr::null_mut(), NonNull::as_ptr) };

        while !cur.is_null() {
            // SAFETY: as above.
            let node_key = unsafe { key_of(cur) };
            if node_key == key {
                return cur;
            }
            let side = if key < node_key {
                RBTREE_LEFT
            } else {
                RBTREE_RIGHT
            };
            // SAFETY: the caller promises valid links.
            cur = unsafe {
                (*cur).children[side as usize]
                    .map_or(ptr::null_mut(), NonNull::as_ptr)
            };
        }

        ptr::null_mut()
    }

    /// The keys in order, with an independent traversal so a broken
    /// link or walk cannot agree with itself.
    ///
    /// # Safety
    ///
    /// `tree` must be valid.
    unsafe fn keys_in_order(tree: *mut Rbtree) -> Vec<u32> {
        let mut keys = Vec::new();
        // SAFETY: the caller promises a valid tree.
        unsafe { collect_in_order((*tree).root, &mut keys) };
        keys
    }

    /// # Safety
    ///
    /// `node` must be null or a valid node.
    unsafe fn collect_in_order(
        node: Option<NonNull<RbtreeNode>>,
        keys: &mut Vec<u32>,
    ) {
        if let Some(node) = node {
            // SAFETY: the caller promises a valid node.
            let children = unsafe { (*node.as_ptr()).children };
            // SAFETY: the caller promises valid links.
            unsafe { collect_in_order(children[LEFT], keys) };
            // SAFETY: the caller promises a node inside an Entry.
            keys.push(unsafe { key_of(node.as_ptr()) });
            // SAFETY: as above.
            unsafe { collect_in_order(children[RIGHT], keys) };
        }
    }

    /// Verify the red-black rules and parent links; returns the black
    /// height.
    ///
    /// # Safety
    ///
    /// `tree` must be valid.
    unsafe fn check(tree: *mut Rbtree) -> usize {
        // SAFETY: the caller promises a valid tree.
        match unsafe { (*tree).root() } {
            None => 1,
            Some(root) => {
                // SAFETY: as above.
                assert_eq!(unsafe { root.parent() }, None, "root parent");
                assert!(unsafe { root.is_black() }, "root is black");
                unsafe { check_node(root) }
            }
        }
    }

    /// # Safety
    ///
    /// `node` must be a valid node.
    unsafe fn check_node(node: NodeRef) -> usize {
        // SAFETY: the caller promises a valid node.
        let children =
            unsafe { [node.child(Side::Left), node.child(Side::Right)] };

        // SAFETY: as above.
        for child in children {
            if let Some(child) = child {
                // SAFETY: the links of a valid node point at nodes.
                assert_eq!(
                    unsafe { child.parent() },
                    Some(node),
                    "parent link"
                );
            }
        }

        // SAFETY: the caller promises a valid node.
        if unsafe { node.is_red() } {
            for child in children {
                // SAFETY: as above.
                assert!(
                    child.is_none_or(|c| unsafe { c.is_black() }),
                    "red node with a red child"
                );
            }
        }

        // SAFETY: the caller promises valid links.
        let left = children[LEFT].map_or(1, |c| unsafe { check_node(c) });
        // SAFETY: as above.
        let right = children[RIGHT].map_or(1, |c| unsafe { check_node(c) });
        assert_eq!(left, right, "black height");

        // SAFETY: the caller promises a valid node.
        if unsafe { node.is_red() } {
            left
        } else {
            left + 1
        }
    }

    struct Rng(u32);

    impl Rng {
        fn next(&mut self) -> u32 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            self.0 = x;
            x
        }
    }

    /// A Fisher-Yates shuffle of `keys`, so the mutation order is
    /// random but reproducible.
    fn shuffled(keys: &[u32], rng: &mut Rng) -> Vec<u32> {
        let mut v = keys.to_vec();
        for i in (1..v.len()).rev() {
            let j = (rng.next() as usize) % (i + 1);
            v.swap(i, j);
        }
        v
    }

    #[test]
    fn empty_tree() {
        let mut tree = new_tree();

        // SAFETY: the tree is valid, and null arguments are allowed.
        unsafe {
            assert!(rbtree_firstlast(&tree, RBTREE_LEFT).is_null());
            assert!(rbtree_firstlast(&tree, RBTREE_RIGHT).is_null());
            assert!(
                rbtree_nearest(ptr::null_mut(), -1, RBTREE_LEFT).is_null()
            );
            assert_eq!(rbtree_slot(ptr::null_mut(), 0), 0);
            assert!(keys_in_order(&mut tree).is_empty());
            assert_eq!(check(&mut tree), 1);
        }
    }

    #[test]
    fn init_empties_tree() {
        let v = entries(&[1, 2, 3]);
        let mut tree = new_tree();

        // SAFETY: `v` is stable, each entry inserted once.
        unsafe {
            for entry in &v {
                insert(&mut tree, entry);
            }
        }
        assert_eq!(unsafe { keys_in_order(&mut tree) }, [1, 2, 3]);

        tree.init();
        assert!(tree.root.is_none());
        assert!(unsafe { keys_in_order(&mut tree) }.is_empty());

        // A tree that was reset accepts fresh nodes.
        let fresh = entries(&[4]);
        // SAFETY: `fresh` is stable and unlinked.
        unsafe {
            insert(&mut tree, &fresh[0]);
            check(&mut tree);
        }
        assert_eq!(unsafe { keys_in_order(&mut tree) }, [4]);
    }

    #[test]
    fn single_node() {
        let v = entries(&[7]);
        let mut tree = new_tree();

        // SAFETY: one entry, inserted once, then removed.
        unsafe {
            insert(&mut tree, &v[0]);
            let node = node_of(&v[0]);

            assert_eq!(rbtree_firstlast(&tree, RBTREE_LEFT), node);
            assert_eq!(rbtree_firstlast(&tree, RBTREE_RIGHT), node);
            assert_eq!(keys_in_order(&mut tree), [7]);
            check(&mut tree);

            rbtree_remove(&mut tree, node);
            assert!(tree.root.is_none());
            // The removed node is stale, not relinked to itself.
            assert_eq!(NodeRef::new(node).parent(), None);
            assert_eq!(check(&mut tree), 1);
        }
    }

    #[test]
    fn node_colors() {
        let v = entries(&[1]);
        let mut tree = new_tree();

        // SAFETY: one entry, inserted once.
        unsafe {
            // A fresh node is red; the root turns black on insertion.
            let node = NodeRef::new(node_of(&v[0]));
            assert_eq!(node.color(), Color::Red);
            insert(&mut tree, &v[0]);
            assert_eq!(tree.root().unwrap().color(), Color::Black);
        }
    }

    #[test]
    fn ascending_and_descending() {
        let keys: Vec<u32> = (0..128).collect();

        let v = entries(&keys);
        let mut tree = new_tree();
        // SAFETY: `v` is stable, each entry inserted once.
        unsafe {
            for entry in &v {
                insert(&mut tree, entry);
                check(&mut tree);
            }
            assert_eq!(keys_in_order(&mut tree), keys);
            assert_eq!(rbtree_firstlast(&tree, RBTREE_LEFT), node_of(&v[0]));
            assert_eq!(
                rbtree_firstlast(&tree, RBTREE_RIGHT),
                node_of(&v[127])
            );
        }

        let reversed = entries(&keys);
        let mut tree = new_tree();
        // SAFETY: as above, with fresh entries.
        unsafe {
            for entry in reversed.iter().rev() {
                insert(&mut tree, entry);
                check(&mut tree);
            }
            assert_eq!(keys_in_order(&mut tree), keys);
        }
    }

    #[test]
    fn random_insert_remove() {
        let count = 257u32;
        let keys: Vec<u32> = (0..count).collect();
        let mut rng = Rng(0x1234_5678);
        let v = entries(&keys);
        let mut tree = new_tree();
        let mut model: Vec<u32> = Vec::new();

        for &key in &shuffled(&keys, &mut rng) {
            // SAFETY: `v` is stable, each entry inserted once.
            unsafe { insert(&mut tree, &v[key as usize]) };
            let pos = model.partition_point(|&k| k < key);
            model.insert(pos, key);
            // SAFETY: the tree is valid.
            unsafe { check(&mut tree) };
            assert_eq!(unsafe { keys_in_order(&mut tree) }, model);
        }

        // The nearest node, on both sides of every key.
        for probe in 0..count + 2 {
            let expected = match model.binary_search(&probe) {
                Ok(_) => (Some(probe), Some(probe)),
                Err(pos) => (
                    pos.checked_sub(1).map(|i| model[i]),
                    model.get(pos).copied(),
                ),
            };
            // SAFETY: the tree is valid.
            let (prev, next) = unsafe {
                (
                    lookup_nearest(&mut tree, probe, RBTREE_LEFT),
                    lookup_nearest(&mut tree, probe, RBTREE_RIGHT),
                )
            };
            let prev = if prev.is_null() {
                None
            } else {
                // SAFETY: the probe returned a linked node.
                Some(unsafe { key_of(prev) })
            };
            let next = if next.is_null() {
                None
            } else {
                // SAFETY: as above.
                Some(unsafe { key_of(next) })
            };
            assert_eq!(prev, expected.0, "previous of {probe}");
            assert_eq!(next, expected.1, "next of {probe}");

            // The Rust-native method walks the same tree with the same
            // protocol as `rbtree_lookup_nearest()`.
            let cmp = |node: NonNull<RbtreeNode>| {
                // SAFETY: the method only visits linked nodes.
                probe as c_int - unsafe { key_of(node.as_ptr()) } as c_int
            };
            let method_prev = tree.lookup_nearest(cmp, RBTREE_LEFT);
            let method_next = tree.lookup_nearest(cmp, RBTREE_RIGHT);
            let method_prev =
                method_prev.map(|n| unsafe { key_of(n.as_ptr()) });
            let method_next =
                method_next.map(|n| unsafe { key_of(n.as_ptr()) });
            assert_eq!(method_prev, expected.0, "native previous of {probe}");
            assert_eq!(method_next, expected.1, "native next of {probe}");
        }

        // Remove in another random order, checking after every step.
        for &key in &shuffled(&keys, &mut rng) {
            // SAFETY: the tree still holds every entry.
            let node = unsafe { find(&mut tree, key) };
            assert!(!node.is_null(), "missing key {key}");
            // SAFETY: the node is linked in the tree.
            unsafe { rbtree_remove(&mut tree, node) };
            let pos = model.binary_search(&key).unwrap();
            model.remove(pos);
            // SAFETY: the tree is valid.
            unsafe { check(&mut tree) };
            assert_eq!(unsafe { keys_in_order(&mut tree) }, model);
        }

        assert!(tree.root.is_none());
    }

    #[test]
    fn lookup_slot_matches_insert() {
        let keys: Vec<u32> = (0..64).collect();
        let v = entries(&keys);
        let mut tree = new_tree();
        let mut model: Vec<u32> = Vec::new();
        let mut rng = Rng(0x9e37_79b9);

        for &key in &shuffled(&keys, &mut rng) {
            // SAFETY: `v` is stable and the key is not in the tree yet.
            let (found, slot) = unsafe { lookup_slot(&mut tree, key) };
            assert!(found.is_null(), "key {key} already present");
            // SAFETY: the slot came from this tree.
            unsafe {
                rbtree_insert_slot(&mut tree, slot, node_of(&v[key as usize]))
            };
            let pos = model.partition_point(|&k| k < key);
            model.insert(pos, key);
            // SAFETY: the tree is valid.
            unsafe { check(&mut tree) };
            assert_eq!(unsafe { keys_in_order(&mut tree) }, model);
        }

        for &key in &keys {
            // SAFETY: the tree is valid.
            let (found, _) = unsafe { lookup_slot(&mut tree, key) };
            assert_eq!(found, unsafe { find(&mut tree, key) });
        }
    }

    #[test]
    fn remove_inner_nodes() {
        let keys: Vec<u32> = (1..=15).collect();
        let v = entries(&keys);
        let mut tree = new_tree();
        let mut model = keys.clone();

        // SAFETY: `v` is stable, each entry inserted once.
        unsafe {
            for entry in &v {
                insert(&mut tree, entry);
            }
        }

        // The root first, then every shape of two-children node.
        for &key in &[8u32, 4, 12, 2, 6, 10, 14, 1, 3, 5, 7, 9, 11, 13, 15] {
            // SAFETY: the tree still holds every entry.
            let node = unsafe { find(&mut tree, key) };
            assert!(!node.is_null(), "missing key {key}");
            // SAFETY: the node is linked in the tree.
            unsafe { rbtree_remove(&mut tree, node) };
            model.retain(|&k| k != key);
            // SAFETY: the tree is valid.
            unsafe { check(&mut tree) };
            assert_eq!(unsafe { keys_in_order(&mut tree) }, model);
        }

        assert!(tree.root.is_none());
    }
}
