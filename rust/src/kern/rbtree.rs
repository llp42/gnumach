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
