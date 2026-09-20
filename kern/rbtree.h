/*
 * Copyright (c) 2010, 2011 Richard Braun.
 * All rights reserved.
 *
 * Redistribution and use in source and binary forms, with or without
 * modification, are permitted provided that the following conditions
 * are met:
 * 1. Redistributions of source code must retain the above copyright
 *    notice, this list of conditions and the following disclaimer.
 * 2. Redistributions in binary form must reproduce the above copyright
 *    notice, this list of conditions and the following disclaimer in the
 *    documentation and/or other materials provided with the distribution.
 *
 * THIS SOFTWARE IS PROVIDED BY THE AUTHOR ``AS IS'' AND ANY EXPRESS OR
 * IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES
 * OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED.
 * IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR ANY DIRECT, INDIRECT,
 * INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT
 * NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
 * DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
 * THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
 * (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF
 * THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
 *
 *
 * Red-black tree.
 */

#ifndef _KERN_RBTREE_H
#define _KERN_RBTREE_H

#include <stddef.h>
#include <kern/macros.h>
#include <sys/types.h>

/*
 * Indexes of the left and right nodes in the children array of a node.
 */
#define RBTREE_LEFT     0
#define RBTREE_RIGHT    1

/*
 * Red-black node.
 */
struct rbtree_node;

/*
 * Red-black tree.
 */
struct rbtree;

#include "rbtree_i.h"

/*
 * rbtree_init(), rbtree_node_init(), rbtree_insert_slot(), rbtree_slot()
 * and rbtree_d2i() live in rust/src/kern/rbtree.rs and rbtree_i.h
 * declares them.
 *
 * A node is in no tree when its parent points to itself.
 */

/*
 * Macro that evaluates to the address of the structure containing the
 * given node based on the given type and member.
 */
#define rbtree_entry(node, type, member) structof(node, type, member)

/*
 * Look up a node or one of its nearest nodes in a tree.
 *
 * This macro essentially acts as rbtree_lookup() but if no entry matched
 * the key, an additional step is performed to obtain the next or previous
 * node, depending on the direction (left or right).
 *
 * The constraints that apply to the key parameter are the same as for
 * rbtree_lookup().
 */
#define rbtree_lookup_nearest(tree, key, cmp_fn, dir)       \
MACRO_BEGIN                                                 \
    struct rbtree_node *___cur, *___prev;                   \
    int ___diff, ___index;                                  \
                                                            \
    ___prev = NULL;                                         \
    ___index = -1;                                          \
    ___cur = (tree)->root;                                  \
                                                            \
    while (___cur != NULL) {                                \
        ___diff = cmp_fn(key, ___cur);                      \
                                                            \
        if (___diff == 0)                                   \
            break;                                          \
                                                            \
        ___prev = ___cur;                                   \
        ___index = rbtree_d2i(___diff);                     \
        ___cur = ___cur->children[___index];                \
    }                                                       \
                                                            \
    if (___cur == NULL)                                     \
        ___cur = rbtree_nearest(___prev, ___index, dir);    \
                                                            \
    ___cur;                                                 \
MACRO_END

/*
 * Insert a node in a tree.
 *
 * This macro performs a standard lookup to obtain the insertion point of
 * the given node in the tree (it is assumed that the inserted node never
 * compares equal to any other entry in the tree) and links the node. It
 * then checks red-black rules violations, and rebalances the tree if
 * necessary.
 *
 * Unlike rbtree_lookup(), the cmp_fn parameter must compare two complete
 * entries, so it is suggested to use two different comparison inline
 * functions, such as myobj_cmp_lookup() and myobj_cmp_insert(). There is no
 * guarantee about the order of the nodes given to the comparison function.
 *
 * See rbtree_lookup().
 */
#define rbtree_insert(tree, node, cmp_fn)                   \
MACRO_BEGIN                                                 \
    struct rbtree_node *___cur, *___prev;                   \
    int ___diff, ___index;                                  \
                                                            \
    ___prev = NULL;                                         \
    ___index = -1;                                          \
    ___cur = (tree)->root;                                  \
                                                            \
    while (___cur != NULL) {                                \
        ___diff = cmp_fn(node, ___cur);                     \
        ___prev = ___cur;                                   \
        ___index = rbtree_d2i(___diff);                     \
        ___cur = ___cur->children[___index];                \
    }                                                       \
                                                            \
    rbtree_insert_rebalance(tree, ___prev, ___index, node); \
MACRO_END

/*
 * Look up a node/slot pair in a tree.
 *
 * This macro essentially acts as rbtree_lookup() but in addition to a node,
 * it also returns a slot, which identifies an insertion point in the tree.
 * If the returned node is null, the slot can be used by rbtree_insert_slot()
 * to insert without the overhead of an additional lookup. The slot is a
 * simple unsigned long integer.
 *
 * The constraints that apply to the key parameter are the same as for
 * rbtree_lookup().
 */
#define rbtree_lookup_slot(tree, key, cmp_fn, slot) \
MACRO_BEGIN                                         \
    struct rbtree_node *___cur, *___prev;           \
    int ___diff, ___index;                          \
                                                    \
    ___prev = NULL;                                 \
    ___index = 0;                                   \
    ___cur = (tree)->root;                          \
                                                    \
    while (___cur != NULL) {                        \
        ___diff = cmp_fn(key, ___cur);              \
                                                    \
        if (___diff == 0)                           \
            break;                                  \
                                                    \
        ___prev = ___cur;                           \
        ___index = rbtree_d2i(___diff);             \
        ___cur = ___cur->children[___index];        \
    }                                               \
                                                    \
    (slot) = rbtree_slot(___prev, ___index);        \
    ___cur;                                         \
MACRO_END

/*
 * Insert a node at an insertion point in a tree.
 *
 * This macro essentially acts as rbtree_insert() except that it doesn't
 * obtain the insertion point with a standard lookup. The insertion point
 * is obtained by calling rbtree_lookup_slot(). In addition, the new node
 * must not compare equal to an existing node in the tree (i.e. the slot
 * must denote a null node).
 *
 * Defined in rust/src/kern/rbtree.rs; rbtree_i.h declares it.
 */

/*
 * Remove a node from a tree.
 *
 * After completion, the node is stale.
 */
void rbtree_remove(struct rbtree *tree, struct rbtree_node *node);

/*
 * Return the first node of a tree.
 */
#define rbtree_first(tree) rbtree_firstlast(tree, RBTREE_LEFT)

/*
 * Return the last node of a tree.
 */
#define rbtree_last(tree) rbtree_firstlast(tree, RBTREE_RIGHT)

#endif /* _KERN_RBTREE_H */
