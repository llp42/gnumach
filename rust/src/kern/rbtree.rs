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

/// `RBTREE_COLOR_*` and the parent masks of <kern/rbtree_i.h>.
const COLOR_MASK: usize = 0x1;
const PARENT_MASK: usize = !0x3;
const COLOR_RED: c_int = 0;
const COLOR_BLACK: c_int = 1;

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

/// View a raw node the caller promises is non-null.
///
/// # Safety
///
/// `node` must not be null; the C callers dereference it right away.
unsafe fn non_null<T>(ptr: *mut T) -> NonNull<T> {
    // SAFETY: the caller promises the pointer is non-null.
    unsafe { NonNull::new_unchecked(ptr) }
}

/// The parent address without the color bit.
///
/// # Safety
///
/// `node` must point at a valid node.
unsafe fn get_parent(
    node: NonNull<RbtreeNode>,
) -> Option<NonNull<RbtreeNode>> {
    // SAFETY: the caller promises a valid node.
    let addr = unsafe { (*node.as_ptr()).parent } & PARENT_MASK;
    NonNull::new(ptr::with_exposed_provenance_mut(addr))
}

/// The color bit of a node.
///
/// # Safety
///
/// `node` must point at a valid node.
unsafe fn get_color(node: NonNull<RbtreeNode>) -> c_int {
    // SAFETY: the caller promises a valid node.
    unsafe { ((*node.as_ptr()).parent & COLOR_MASK) as c_int }
}

/// Whether a node is red.
///
/// # Safety
///
/// `node` must point at a valid node.
unsafe fn is_red(node: NonNull<RbtreeNode>) -> bool {
    // SAFETY: the caller promises a valid node.
    unsafe { get_color(node) == COLOR_RED }
}

/// Whether a node is black.
///
/// # Safety
///
/// `node` must point at a valid node.
unsafe fn is_black(node: NonNull<RbtreeNode>) -> bool {
    // SAFETY: the caller promises a valid node.
    unsafe { get_color(node) == COLOR_BLACK }
}

/// Set the parent, retaining the color.  `rbtree_set_parent()` in C.
///
/// # Safety
///
/// `node` must point at a valid node.
unsafe fn set_parent(
    node: NonNull<RbtreeNode>,
    new_parent: Option<NonNull<RbtreeNode>>,
) {
    // SAFETY: the caller promises a valid node.
    unsafe {
        let addr = new_parent.map_or(0, |p| p.as_ptr().addr());
        let node = node.as_ptr();
        (*node).parent = addr | ((*node).parent & COLOR_MASK);
    }
}

/// Set the color, retaining the parent.  `rbtree_set_color()` in C.
///
/// # Safety
///
/// `node` must point at a valid node.
unsafe fn set_color(node: NonNull<RbtreeNode>, new_color: c_int) {
    // SAFETY: the caller promises a valid node.
    unsafe {
        let node = node.as_ptr();
        (*node).parent = ((*node).parent & PARENT_MASK) | new_color as usize;
    }
}

/// Paint a node red.
///
/// # Safety
///
/// `node` must point at a valid node.
unsafe fn set_red(node: NonNull<RbtreeNode>) {
    // SAFETY: the caller promises a valid node.
    unsafe { set_color(node, COLOR_RED) };
}

/// Paint a node black.
///
/// # Safety
///
/// `node` must point at a valid node.
unsafe fn set_black(node: NonNull<RbtreeNode>) {
    // SAFETY: the caller promises a valid node.
    unsafe { set_color(node, COLOR_BLACK) };
}

/// Index of a node, or of a null child, in its parent's children.
///
/// A null child counts as the left one when the left link is null;
/// `rbtree_index()` in C works the same way, and the remove path
/// depends on it.
///
/// # Safety
///
/// `parent` must point at a valid node, and `node` must be one of its
/// children (or null where one child is null).
unsafe fn child_index(
    node: Option<NonNull<RbtreeNode>>,
    parent: NonNull<RbtreeNode>,
) -> c_int {
    // SAFETY: the caller promises a valid parent.
    if unsafe { (*parent.as_ptr()).children[LEFT] } == node {
        RBTREE_LEFT
    } else {
        RBTREE_RIGHT
    }
}

/// Rotate the tree rooted at `node` in `direction`.  `rbtree_rotate()`
/// in C.
///
/// # Safety
///
/// `tree` must be valid, `node` must be linked in it, and the child on
/// the opposite side must exist.
unsafe fn rotate(
    tree: NonNull<Rbtree>,
    node: NonNull<RbtreeNode>,
    direction: c_int,
) {
    let left = direction as usize;
    let right = 1 - left;
    // SAFETY: the caller promises a linked node with the right subtree.
    let parent = unsafe { get_parent(node) };
    let rnode = unsafe { (*node.as_ptr()).children[right] }
        .expect("rbtree: rotate at a node without its child");

    // SAFETY: the caller promises all of these nodes are valid.
    unsafe {
        (*node.as_ptr()).children[right] = (*rnode.as_ptr()).children[left];

        if let Some(child) = (*rnode.as_ptr()).children[left] {
            set_parent(child, Some(node));
        }

        (*rnode.as_ptr()).children[left] = Some(node);
        set_parent(rnode, parent);

        match parent {
            None => (*tree.as_ptr()).root = Some(rnode),
            Some(p) => {
                let index = child_index(Some(node), p);
                (*p.as_ptr()).children[index as usize] = Some(rnode);
            }
        }

        set_parent(node, Some(rnode));
    }
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
    // SAFETY: the caller's macro passes the tree, the found parent (or
    // null) and the caller-owned node.
    let tree = unsafe { non_null(tree) };
    let mut node = unsafe { non_null(node) };
    let mut parent = NonNull::new(parent);

    // SAFETY: the caller promises valid nodes and an unlinked node.
    unsafe {
        (*node.as_ptr()).parent =
            parent.map_or(0, |p| p.as_ptr().addr()) | COLOR_RED as usize;
        (*node.as_ptr()).children = [None, None];

        match parent {
            None => (*tree.as_ptr()).root = Some(node),
            Some(p) => (*p.as_ptr()).children[index as usize] = Some(node),
        }
    }

    loop {
        let Some(p) = parent else {
            // SAFETY: the caller promises a valid node.
            unsafe { set_black(node) };
            break;
        };

        // SAFETY: the caller promises valid nodes.
        if unsafe { is_black(p) } {
            break;
        }

        // A red node always has a parent in a valid tree.
        // SAFETY: as above.
        let grand_parent = unsafe { get_parent(p) }
            .expect("rbtree: red node without a grandparent");
        // SAFETY: as above.
        let left = unsafe { child_index(Some(p), grand_parent) } as usize;
        let right = 1 - left;
        // SAFETY: as above.
        let uncle = unsafe { (*grand_parent.as_ptr()).children[right] };

        // Uncle is red: flip colors and repeat at the grandparent.
        if let Some(uncle) = uncle {
            // SAFETY: as above.
            if unsafe { is_red(uncle) } {
                // SAFETY: as above.
                unsafe {
                    set_black(uncle);
                    set_black(p);
                    set_red(grand_parent);
                }
                node = grand_parent;
                parent = unsafe { get_parent(node) };
                continue;
            }
        }

        // Node is the right child of its parent: rotate left at the
        // parent and swap the two.
        // SAFETY: as above.
        if unsafe { (*p.as_ptr()).children[right] } == Some(node) {
            // SAFETY: as above.
            unsafe { rotate(tree, p, left as c_int) };
            // The old node takes the parent's place for the last step.
            parent = Some(node);
        }

        // Node is the left child: recolor, rotate right at the
        // grandparent, and leave.
        let p = parent.expect("rbtree: red node without a parent");
        // SAFETY: as above.
        unsafe {
            set_black(p);
            set_red(grand_parent);
            rotate(tree, grand_parent, right as c_int);
        }
        break;
    }
}

/// Remove a node from a tree.  `rbtree_remove()` in C.
///
/// After completion the node is stale: its links are not cleared, as
/// the C comment in <kern/rbtree.h> says.
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
    let tree = unsafe { non_null(tree) };
    let node = unsafe { non_null(node) };

    let mut child: Option<NonNull<RbtreeNode>>;
    let mut parent: Option<NonNull<RbtreeNode>>;
    let color: c_int;

    // SAFETY: the caller promises valid nodes.
    let (left, right) = unsafe {
        (
            (*node.as_ptr()).children[LEFT],
            (*node.as_ptr()).children[RIGHT],
        )
    };

    if left.is_none() {
        // Node has at most one child.
        child = right;
        // SAFETY: as above.
        color = unsafe { get_color(node) };
        parent = unsafe { get_parent(node) };

        // SAFETY: as above.
        unsafe {
            if let Some(c) = child {
                set_parent(c, parent);
            }

            match parent {
                None => (*tree.as_ptr()).root = child,
                Some(p) => {
                    let index = child_index(Some(node), p);
                    (*p.as_ptr()).children[index as usize] = child;
                }
            }
        }
    } else if right.is_none() {
        child = left;
        // SAFETY: as above.
        color = unsafe { get_color(node) };
        parent = unsafe { get_parent(node) };

        // SAFETY: as above.
        unsafe {
            if let Some(c) = child {
                set_parent(c, parent);
            }

            match parent {
                None => (*tree.as_ptr()).root = child,
                Some(p) => {
                    let index = child_index(Some(node), p);
                    (*p.as_ptr()).children[index as usize] = child;
                }
            }
        }
    } else {
        // Two children: replace the node with its successor.
        let mut successor = right.expect("rbtree: right child");
        // SAFETY: as above.
        while let Some(l) = unsafe { (*successor.as_ptr()).children[LEFT] } {
            successor = l;
        }

        // SAFETY: as above.
        color = unsafe { get_color(successor) };
        child = unsafe { (*successor.as_ptr()).children[RIGHT] };
        parent = unsafe { get_parent(node) };

        // SAFETY: as above.
        unsafe {
            match parent {
                None => (*tree.as_ptr()).root = Some(successor),
                Some(p) => {
                    let index = child_index(Some(node), p);
                    (*p.as_ptr()).children[index as usize] = Some(successor);
                }
            }
        }

        // The successor's original parent decides the next step.
        // SAFETY: as above.
        let successor_parent = unsafe { get_parent(successor) };

        // Set parent directly to keep the original color.
        // SAFETY: as above.
        unsafe {
            (*successor.as_ptr()).parent = (*node.as_ptr()).parent;
            (*successor.as_ptr()).children[LEFT] =
                (*node.as_ptr()).children[LEFT];
            if let Some(l) = (*successor.as_ptr()).children[LEFT] {
                set_parent(l, Some(successor));
            }
        }

        if Some(node) == successor_parent {
            parent = Some(successor);
        } else {
            let successor_parent =
                successor_parent.expect("rbtree: successor without a parent");
            // SAFETY: as above.
            unsafe {
                (*successor.as_ptr()).children[RIGHT] =
                    (*node.as_ptr()).children[RIGHT];
                if let Some(r) = (*successor.as_ptr()).children[RIGHT] {
                    set_parent(r, Some(successor));
                }

                (*successor_parent.as_ptr()).children[LEFT] = child;
                if let Some(c) = child {
                    set_parent(c, Some(successor_parent));
                }
            }
            parent = Some(successor_parent);
        }
    }

    // Update the colors; a null child counts as a black leaf.
    if color == COLOR_RED {
        return;
    }

    loop {
        if let Some(c) = child {
            // SAFETY: the caller promises valid links.
            if unsafe { is_red(c) } {
                // SAFETY: as above.
                unsafe { set_black(c) };
                break;
            }
        }

        let Some(p) = parent else {
            break;
        };

        // SAFETY: as above.
        let left = unsafe { child_index(child, p) } as usize;
        let right = 1 - left;
        // SAFETY: as above.
        let mut brother = unsafe { (*p.as_ptr()).children[right] }
            .expect("rbtree: black node without a brother");

        // Brother is red: recolor and rotate left at the parent so that
        // the brother becomes black.
        // SAFETY: as above.
        if unsafe { is_red(brother) } {
            // SAFETY: as above.
            unsafe {
                set_black(brother);
                set_red(p);
                rotate(tree, p, left as c_int);
            }
            brother = unsafe { (*p.as_ptr()).children[right] }
                .expect("rbtree: black node without a brother");
        }

        // SAFETY: as above.
        let brother_left = unsafe { (*brother.as_ptr()).children[left] };
        let brother_right = unsafe { (*brother.as_ptr()).children[right] };

        // Brother has no red child: recolor and repeat at the parent.
        let left_black = brother_left.is_none_or(|n| unsafe { is_black(n) });
        let right_black = brother_right.is_none_or(|n| unsafe { is_black(n) });
        if left_black && right_black {
            // SAFETY: as above.
            unsafe { set_red(brother) };
            child = Some(p);
            parent = unsafe { get_parent(p) };
            continue;
        }

        // Brother's right child is black: recolor and rotate right at
        // the brother.
        if right_black {
            let brother_left =
                brother_left.expect("rbtree: black node without a red child");
            // SAFETY: as above.
            unsafe {
                set_black(brother_left);
                set_red(brother);
                rotate(tree, brother, right as c_int);
            }
            brother = unsafe { (*p.as_ptr()).children[right] }
                .expect("rbtree: black node without a brother");
        }

        // Exchange the parent and brother colors, blacken the brother's
        // right child, rotate left at the parent, and leave.
        // SAFETY: as above.
        unsafe {
            set_color(brother, get_color(p));
            set_black(p);
            let brother_right = (*brother.as_ptr()).children[right]
                .expect("rbtree: brother without a right child");
            set_black(brother_right);
            rotate(tree, p, left as c_int);
        }
        break;
    }
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
    let Some(parent) = NonNull::new(parent) else {
        return ptr::null_mut();
    };

    if index != direction {
        parent.as_ptr()
    } else {
        // SAFETY: the caller promises a valid node.
        unsafe { rbtree_walk(parent.as_ptr(), direction) }
    }
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
    let mut prev: Option<NonNull<RbtreeNode>> = None;
    // SAFETY: the caller promises a valid tree.
    let mut cur = unsafe { (*tree).root };

    while let Some(node) = cur {
        prev = Some(node);
        // SAFETY: as above.
        cur = unsafe { (*node.as_ptr()).children[direction as usize] };
    }

    prev.map_or(ptr::null_mut(), NonNull::as_ptr)
}

/// The node next to, or previous to, a node.  `rbtree_walk()` in C.
///
/// # Safety
///
/// `node` must be null or a valid, linked node.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rbtree_walk(
    node: *mut RbtreeNode,
    direction: c_int,
) -> *mut RbtreeNode {
    let left = direction as usize;
    let right = 1 - left;

    let mut node = match NonNull::new(node) {
        None => return ptr::null_mut(),
        Some(node) => node,
    };

    // SAFETY: the caller promises a valid node.
    if let Some(mut child) = unsafe { (*node.as_ptr()).children[left] } {
        loop {
            // SAFETY: as above.
            match unsafe { (*child.as_ptr()).children[right] } {
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
            let Some(parent) = (unsafe { get_parent(node) }) else {
                return ptr::null_mut();
            };
            // SAFETY: as above.
            let index = unsafe { child_index(Some(node), parent) };
            node = parent;

            if index as usize == right {
                break;
            }
        }
    }

    node.as_ptr()
}

/// The left-most deepest node of `node`.  `rbtree_find_deepest()` in C.
///
/// # Safety
///
/// `node` must be a valid node.
unsafe fn find_deepest(mut node: NonNull<RbtreeNode>) -> NonNull<RbtreeNode> {
    loop {
        let parent = node;
        // SAFETY: the caller promises a valid node.
        node = match unsafe { (*node.as_ptr()).children[LEFT] } {
            Some(child) => child,
            None => {
                // SAFETY: as above.
                match unsafe { (*parent.as_ptr()).children[RIGHT] } {
                    Some(child) => child,
                    None => return parent,
                }
            }
        };
    }
}

/// The start of a postorder traversal.  `rbtree_postwalk_deepest()` in C.
///
/// # Safety
///
/// `tree` must be a valid tree.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rbtree_postwalk_deepest(
    tree: *const Rbtree,
) -> *mut RbtreeNode {
    // SAFETY: the caller promises a valid tree.
    match unsafe { (*tree).root } {
        None => ptr::null_mut(),
        // SAFETY: as above.
        Some(node) => unsafe { find_deepest(node) }.as_ptr(),
    }
}

/// Unlink a node and return the next node in postorder.
/// `rbtree_postwalk_unlink()` in C.
///
/// # Safety
///
/// `node` must be null or a valid node whose left subtree has already
/// been unlinked by the traversal.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rbtree_postwalk_unlink(
    node: *mut RbtreeNode,
) -> *mut RbtreeNode {
    let Some(node) = NonNull::new(node) else {
        return ptr::null_mut();
    };
    // SAFETY: the caller promises a valid, linked node.
    let Some(parent) = (unsafe { get_parent(node) }) else {
        return ptr::null_mut();
    };

    // SAFETY: as above.
    unsafe {
        let index = child_index(Some(node), parent);
        (*parent.as_ptr()).children[index as usize] = None;

        match (*parent.as_ptr()).children[RIGHT] {
            None => parent.as_ptr(),
            Some(node) => find_deepest(node).as_ptr(),
        }
    }
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
            v.push(Entry {
                node: RbtreeNode {
                    parent: 0,
                    children: [None, None],
                },
                key,
            });
        }
        v
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

    /// `rbtree_d2i()` of <kern/rbtree_i.h>.
    fn d2i(diff: i64) -> c_int {
        if diff <= 0 { RBTREE_LEFT } else { RBTREE_RIGHT }
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
            let diff = entry.key as i64 - unsafe { key_of(cur) } as i64;
            prev = cur;
            index = d2i(diff);
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
            let diff = key as i64 - unsafe { key_of(cur) } as i64;
            if diff == 0 {
                break;
            }
            prev = cur;
            index = d2i(diff);
            cur = unsafe {
                (*cur).children[index as usize]
                    .map_or(ptr::null_mut(), NonNull::as_ptr)
            };
        }

        (cur, prev.addr() | index as usize)
    }

    /// The C `rbtree_insert_slot()` inline: unpack the slot and
    /// rebalance.
    ///
    /// # Safety
    ///
    /// The slot must come from `lookup_slot` on this tree and the key
    /// must be absent.
    unsafe fn insert_slot(
        tree: *mut Rbtree,
        slot: usize,
        node: *mut RbtreeNode,
    ) {
        let parent = ptr::with_exposed_provenance_mut(slot & !1usize);
        let index = (slot & 1) as c_int;
        // SAFETY: the caller promises a valid slot and node.
        unsafe { rbtree_insert_rebalance(tree, parent, index, node) };
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
            let diff = key as i64 - unsafe { key_of(cur) } as i64;
            if diff == 0 {
                break;
            }
            prev = cur;
            index = d2i(diff);
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
            cur = unsafe {
                (*cur).children[side as usize]
                    .map_or(ptr::null_mut(), NonNull::as_ptr)
            };
        }

        ptr::null_mut()
    }

    /// The keys in order, by walking first to last.
    ///
    /// # Safety
    ///
    /// `tree` must be valid.
    unsafe fn keys_in_order(tree: *mut Rbtree) -> Vec<u32> {
        let mut keys = Vec::new();
        // SAFETY: the caller promises a valid tree.
        let mut node = unsafe { rbtree_firstlast(tree, RBTREE_LEFT) };

        while !node.is_null() {
            // SAFETY: as above; walk follows linked nodes.
            keys.push(unsafe { key_of(node) });
            node = unsafe { rbtree_walk(node, RBTREE_RIGHT) };
        }

        keys
    }

    /// Verify the red-black rules and parent links; returns the black
    /// height.
    ///
    /// # Safety
    ///
    /// `tree` must be valid.
    unsafe fn check(tree: *mut Rbtree) -> usize {
        // SAFETY: the caller promises a valid tree.
        match unsafe { (*tree).root } {
            None => 1,
            Some(root) => {
                // SAFETY: as above.
                assert_eq!(unsafe { get_parent(root) }, None, "root parent");
                assert!(unsafe { is_black(root) }, "root is black");
                unsafe { check_node(root) }
            }
        }
    }

    /// # Safety
    ///
    /// `node` must be a valid node.
    unsafe fn check_node(node: NonNull<RbtreeNode>) -> usize {
        // SAFETY: the caller promises a valid node.
        let children = unsafe { (*node.as_ptr()).children };

        for child in children {
            if let Some(child) = child {
                // SAFETY: the links of a valid node point at nodes.
                assert_eq!(
                    unsafe { get_parent(child) },
                    Some(node),
                    "parent link"
                );
            }
        }

        if unsafe { is_red(node) } {
            for child in children {
                assert!(
                    child.is_none_or(|c| unsafe { is_black(c) }),
                    "red node with a red child"
                );
            }
        }

        let left = children[LEFT].map_or(1, |c| unsafe { check_node(c) });
        let right = children[RIGHT].map_or(1, |c| unsafe { check_node(c) });
        assert_eq!(left, right, "black height");

        if unsafe { is_red(node) } {
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
        let mut tree = Rbtree { root: None };

        // SAFETY: the tree is valid, and null arguments are allowed.
        unsafe {
            assert!(rbtree_firstlast(&tree, RBTREE_LEFT).is_null());
            assert!(rbtree_firstlast(&tree, RBTREE_RIGHT).is_null());
            assert!(rbtree_walk(ptr::null_mut(), RBTREE_LEFT).is_null());
            assert!(
                rbtree_nearest(ptr::null_mut(), -1, RBTREE_LEFT).is_null()
            );
            assert!(rbtree_postwalk_deepest(&tree).is_null());
            assert!(rbtree_postwalk_unlink(ptr::null_mut()).is_null());
            assert_eq!(check(&mut tree), 1);
        }
    }

    #[test]
    fn single_node() {
        let v = entries(&[7]);
        let mut tree = Rbtree { root: None };

        // SAFETY: one entry, inserted once, then removed.
        unsafe {
            insert(&mut tree, &v[0]);
            let node = node_of(&v[0]);

            assert_eq!(rbtree_firstlast(&tree, RBTREE_LEFT), node);
            assert_eq!(rbtree_firstlast(&tree, RBTREE_RIGHT), node);
            assert!(rbtree_walk(node, RBTREE_LEFT).is_null());
            assert!(rbtree_walk(node, RBTREE_RIGHT).is_null());
            check(&mut tree);

            rbtree_remove(&mut tree, node);
            assert!(tree.root.is_none());
            // The removed node is stale, not relinked to itself.
            assert_ne!((*node).parent & PARENT_MASK, node.addr());
            assert_eq!(check(&mut tree), 1);
        }
    }

    #[test]
    fn ascending_and_descending() {
        let keys: Vec<u32> = (0..128).collect();

        let v = entries(&keys);
        let mut tree = Rbtree { root: None };
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
        let mut tree = Rbtree { root: None };
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
        let mut tree = Rbtree { root: None };
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
                Some(unsafe { key_of(prev) })
            };
            let next = if next.is_null() {
                None
            } else {
                Some(unsafe { key_of(next) })
            };
            assert_eq!(prev, expected.0, "previous of {probe}");
            assert_eq!(next, expected.1, "next of {probe}");
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
        let mut tree = Rbtree { root: None };
        let mut model: Vec<u32> = Vec::new();
        let mut rng = Rng(0x9e37_79b9);

        for &key in &shuffled(&keys, &mut rng) {
            // SAFETY: `v` is stable and the key is not in the tree yet.
            let (found, slot) = unsafe { lookup_slot(&mut tree, key) };
            assert!(found.is_null(), "key {key} already present");
            // SAFETY: the slot came from this tree.
            unsafe { insert_slot(&mut tree, slot, node_of(&v[key as usize])) };
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
        let mut tree = Rbtree { root: None };
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

    #[test]
    fn postwalk_destroys() {
        let keys: Vec<u32> = (0..100).collect();
        let v = entries(&keys);
        let mut tree = Rbtree { root: None };
        let mut rng = Rng(0xdead_beef);

        // SAFETY: `v` is stable, each entry inserted once.
        unsafe {
            for &key in &shuffled(&keys, &mut rng) {
                insert(&mut tree, &v[key as usize]);
            }
            check(&mut tree);
        }

        let mut removed = Vec::new();
        // SAFETY: the tree is valid; the traversal unlinks as it goes.
        let mut node = unsafe { rbtree_postwalk_deepest(&tree) };
        while !node.is_null() {
            removed.push(unsafe { key_of(node) });
            // SAFETY: as above.
            node = unsafe { rbtree_postwalk_unlink(node) };
        }

        removed.sort_unstable();
        assert_eq!(removed, keys);
    }
}
