use crate::{Bounds, Half};
use std::{
    cmp,
    fmt::Debug,
    ops::{Add, Sub},
    ptr::NonNull,
};

/// Maximum children per internal node (R-tree style branching factor).
/// Higher values = shorter tree = fewer cache misses, but more work per node.
const MAX_CHILDREN: usize = 12;

/// A spatial tree optimized for finding maximum ordering among intersecting bounds.
///
/// This is an R-tree variant specifically designed for the use case of assigning
/// z-order to overlapping UI elements. Key optimizations:
/// - Tracks the leaf with global max ordering for O(1) fast-path queries
/// - Uses higher branching factor (4) for lower tree height
/// - Aggressive pruning during search based on max_order metadata
#[derive(Debug)]
pub(crate) struct BoundsTree<U>
where
    U: Clone + Debug + Default + PartialEq,
{
    /// All nodes stored contiguously for cache efficiency.
    nodes: Vec<Node<U>>,
    /// Index of the root node, if any.
    root: Option<usize>,
    /// Index of the leaf with the highest ordering (for fast-path lookups).
    max_leaf: Option<usize>,
    /// Reusable stack for tree traversal during insertion.
    insert_path: Vec<usize>,
    /// Reusable stack for search operations.
    search_stack: Vec<NonNull<Node<U>>>,
}

/// A node in the bounds tree.
#[derive(Debug, Clone)]
struct Node<U>
where
    U: Clone + Debug + Default + PartialEq,
{
    /// Bounding box containing this node and all descendants.
    bounds: Bounds<U>,
    /// Maximum ordering value in this subtree.
    max_order: u32,
    /// Node-specific data.
    kind: NodeKind,
}

#[derive(Debug, Clone)]
enum NodeKind {
    /// Leaf node containing actual bounds data.
    Leaf {
        /// The ordering assigned to this bounds.
        order: u32,
    },
    /// Internal node with children.
    Internal {
        /// Indices of child nodes (2 to MAX_CHILDREN).
        children: NodeChildren,
    },
}

/// Fixed-size array for child indices, avoiding heap allocation.
#[derive(Debug, Clone)]
struct NodeChildren {
    // Keeps an invariant where the max order child is always at the end
    indices: [usize; MAX_CHILDREN],
    len: u8,
}

impl NodeChildren {
    fn new() -> Self {
        Self {
            indices: [0; MAX_CHILDREN],
            len: 0,
        }
    }

    fn push(&mut self, index: usize) {
        debug_assert!((self.len as usize) < MAX_CHILDREN);
        self.indices[self.len as usize] = index;
        self.len += 1;
    }

    fn len(&self) -> usize {
        self.len as usize
    }

    fn as_slice(&self) -> &[usize] {
        &self.indices[..self.len as usize]
    }
}

impl<U> BoundsTree<U>
where
    U: Clone
        + Debug
        + PartialEq
        + PartialOrd
        + Add<U, Output = U>
        + Sub<Output = U>
        + Half
        + Default,
{
    /// Clears all nodes from the tree.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.root = None;
        self.max_leaf = None;
        self.insert_path.clear();
        self.search_stack.clear();
    }

    /// Inserts bounds into the tree and returns its assigned ordering.
    ///
    /// The ordering is one greater than the maximum ordering of any
    /// existing bounds that intersect with the new bounds.
    pub fn insert(&mut self, new_bounds: Bounds<U>) -> u32 {
        // Find maximum ordering among intersecting bounds
        let max_intersecting = self.find_max_ordering(&new_bounds);
        let ordering = max_intersecting + 1;
        self.insert_ordered(new_bounds, ordering);
        ordering
    }

    /// Reserves `[base, base + span]` for a group of pre-ordered bounds, where
    /// `base` is one greater than the maximum ordering among intersecting
    /// bounds. Returns `base`. The tree only records the group's maximum
    /// ordering, so later intersecting bounds sort above the whole group.
    pub(crate) fn insert_group(&mut self, bounds: Bounds<U>, span: u32) -> u32 {
        let base = self.find_max_ordering(&bounds) + 1;
        self.insert_ordered(bounds, base + span);
        base
    }

    fn insert_ordered(&mut self, bounds: Bounds<U>, ordering: u32) {
        let new_leaf_idx = self.insert_leaf(bounds, ordering);

        // Update max_leaf tracking
        self.max_leaf = match self.max_leaf {
            None => Some(new_leaf_idx),
            Some(old_idx) if self.nodes[old_idx].max_order < ordering => Some(new_leaf_idx),
            some => some,
        };
    }

    /// Finds the maximum ordering among all bounds that intersect with the query.
    fn find_max_ordering(&mut self, query: &Bounds<U>) -> u32 {
        let Some(root_idx) = self.root else {
            return 0;
        };

        // Fast path: check if the max-ordering leaf intersects
        if let Some(max_idx) = self.max_leaf {
            let max_node = &self.nodes[max_idx];
            if query.intersects(&max_node.bounds) {
                return max_node.max_order;
            }
        }

        // Slow path: search the tree. Children are spatially filtered before
        // entering the stack, so only the root needs an explicit check.
        if !query.intersects(&self.nodes[root_idx].bounds) {
            return 0;
        }
        self.search_stack.clear();
        self.search_stack.push(NonNull::from(&self.nodes[root_idx]));

        let mut max_found = 0u32;

        while let Some(node) = self.search_stack.pop() {
            // SAFETY: `node` is guaranteed to be valid as the `nodes` stack is unmodified in this function
            // and the `search_stack` only contains pointers from this function call.
            let node = unsafe { node.as_ref() };

            // Pruning: skip if this subtree can't improve our result. This is
            // re-checked because `max_found` can grow while a node waits.
            if node.max_order <= max_found {
                continue;
            }

            match &node.kind {
                NodeKind::Leaf { order } => {
                    max_found = cmp::max(max_found, *order);
                }
                NodeKind::Internal { children } => {
                    // Children are maintained with highest max_order at the end.
                    // Push in forward order to highest (last) is popped first.
                    self.search_stack.extend(
                        children
                            .as_slice()
                            .iter()
                            .map(|&child_idx| &self.nodes[child_idx])
                            .filter(|node| {
                                node.max_order > max_found && query.intersects(&node.bounds)
                            })
                            .map(NonNull::from),
                    );
                }
            }
        }

        max_found
    }

    /// Inserts a leaf node with the given bounds and ordering.
    /// Returns the index of the new leaf.
    fn insert_leaf(&mut self, bounds: Bounds<U>, order: u32) -> usize {
        let new_leaf_idx = self.nodes.len();
        self.nodes.push(Node {
            bounds: bounds.clone(),
            max_order: order,
            kind: NodeKind::Leaf { order },
        });

        let Some(root_idx) = self.root else {
            // Tree is empty, new leaf becomes root
            self.root = Some(new_leaf_idx);
            return new_leaf_idx;
        };

        // If root is a leaf, create internal node with both
        if matches!(self.nodes[root_idx].kind, NodeKind::Leaf { .. }) {
            self.make_root(root_idx, new_leaf_idx);
            return new_leaf_idx;
        }

        // Descend to the parent of the leaf level, choosing the child whose
        // bounds grow the least at each step.
        self.insert_path.clear();
        let mut current_idx = root_idx;
        loop {
            self.insert_path.push(current_idx);
            let best_child_idx = self.best_child(current_idx, &bounds);
            if matches!(self.nodes[best_child_idx].kind, NodeKind::Leaf { .. }) {
                break;
            }
            current_idx = best_child_idx;
        }

        // Attach the new leaf and repair overflows bottom-up. Full nodes are
        // split into siblings instead of nesting new levels, which keeps the
        // tree balanced under monotonic insertion patterns (e.g. long lists).
        let mut path_len = self.insert_path.len();
        let mut pending_child = new_leaf_idx;
        let mut split_child = false;
        loop {
            let node_idx = self.insert_path[path_len - 1];
            let child_count = match &self.nodes[node_idx].kind {
                NodeKind::Internal { children } => children.len(),
                NodeKind::Leaf { .. } => unreachable!("insert path only visits internal nodes"),
            };

            if child_count < MAX_CHILDREN {
                self.attach_child(node_idx, pending_child);
                if split_child {
                    // The reused child kept half of the split's children, so
                    // its bounds and maximum may have changed. The parent has
                    // only seen the new sibling, so recompute from children.
                    self.refresh_node(node_idx);
                }
                let new_bounds = self.nodes[node_idx].bounds.clone();
                let new_max = self.nodes[node_idx].max_order;
                self.grow_ancestors(path_len, &new_bounds, new_max);
                break;
            }

            let sibling_idx = self.split_overflowing(node_idx, pending_child);

            if path_len == 1 {
                self.make_root(node_idx, sibling_idx);
                break;
            }

            path_len -= 1;
            pending_child = sibling_idx;
            split_child = true;
        }

        new_leaf_idx
    }

    /// Returns the child of `node_idx` whose bounds grow least when united with
    /// `bounds`.
    fn best_child(&self, node_idx: usize, bounds: &Bounds<U>) -> usize {
        let NodeKind::Internal { children } = &self.nodes[node_idx].kind else {
            unreachable!("best_child called on a leaf");
        };

        let mut best_child_idx = children.as_slice()[0];
        let mut best_cost = bounds
            .union(&self.nodes[best_child_idx].bounds)
            .half_perimeter();

        for &child_idx in &children.as_slice()[1..] {
            let cost = bounds.union(&self.nodes[child_idx].bounds).half_perimeter();
            if cost < best_cost {
                best_cost = cost;
                best_child_idx = child_idx;
            }
        }

        best_child_idx
    }

    fn attach_child(&mut self, node_idx: usize, child_idx: usize) {
        let child_bounds = self.nodes[child_idx].bounds.clone();
        let child_max = self.nodes[child_idx].max_order;
        let node = &mut self.nodes[node_idx];
        node.bounds = node.bounds.union(&child_bounds);
        let previous_max = node.max_order;
        if child_max > node.max_order {
            node.max_order = child_max;
        }
        if let NodeKind::Internal { children } = &mut node.kind {
            children.push(child_idx);
            // Keep the highest-ordering child at the end. The new child is
            // already at the end, so only the previous max needs moving.
            if child_max <= previous_max {
                let last = children.len() - 1;
                children.indices.swap(last - 1, last);
            }
        }
    }

    /// Grows each ancestor of the just-updated node `insert_path[path_len - 1]`
    /// by the deepest changed bounds. Ancestors already contain the unchanged
    /// children, so a single union per level is sufficient.
    fn grow_ancestors(&mut self, path_len: usize, new_bounds: &Bounds<U>, new_max: u32) {
        for i in (0..path_len - 1).rev() {
            let node_idx = self.insert_path[i];
            let child_idx = self.insert_path[i + 1];
            let node = &mut self.nodes[node_idx];
            node.bounds = node.bounds.union(new_bounds);
            if new_max > node.max_order {
                node.max_order = new_max;
                if let NodeKind::Internal { children } = &mut node.kind {
                    if let Some(pos) = children.as_slice().iter().position(|&c| c == child_idx) {
                        let last = children.len() - 1;
                        if pos != last {
                            children.indices.swap(pos, last);
                        }
                    }
                }
            }
        }
    }

    /// Recomputes a node's bounds and maximum ordering from its children,
    /// keeping the highest-ordering child at the end of the child list.
    fn refresh_node(&mut self, node_idx: usize) {
        let mut child_indices = [0usize; MAX_CHILDREN];
        let len = match &self.nodes[node_idx].kind {
            NodeKind::Internal { children } => {
                let len = children.len();
                child_indices[..len].copy_from_slice(children.as_slice());
                len
            }
            NodeKind::Leaf { .. } => unreachable!("refresh_node called on a leaf"),
        };

        let mut bounds = self.nodes[child_indices[0]].bounds.clone();
        let mut max_order = 0;
        for &child_idx in &child_indices[..len] {
            bounds = bounds.union(&self.nodes[child_idx].bounds);
            max_order = cmp::max(max_order, self.nodes[child_idx].max_order);
        }

        let mut max_pos = 0;
        for (pos, &child_idx) in child_indices[..len].iter().enumerate() {
            if self.nodes[child_idx].max_order == max_order {
                max_pos = pos;
                break;
            }
        }
        if max_pos != len - 1 {
            child_indices.swap(max_pos, len - 1);
        }

        if let NodeKind::Internal { children } = &mut self.nodes[node_idx].kind {
            children.indices[..len].copy_from_slice(&child_indices[..len]);
        }
        self.nodes[node_idx].bounds = bounds;
        self.nodes[node_idx].max_order = max_order;
    }

    /// Splits an overflowing node's children plus `extra_idx` into two nodes
    /// using a linear split, returning the index of the new sibling.
    fn split_overflowing(&mut self, node_idx: usize, extra_idx: usize) -> usize {
        let mut children = [0usize; MAX_CHILDREN + 1];
        let mut len = 0;
        if let NodeKind::Internal { children: existing } = &self.nodes[node_idx].kind {
            for &child_idx in existing.as_slice() {
                children[len] = child_idx;
                len += 1;
            }
        }
        children[len] = extra_idx;
        len += 1;
        debug_assert_eq!(len, MAX_CHILDREN + 1);

        // Pick the axis with the greatest spread of child centers.
        let first_center = self.nodes[children[0]].bounds.center();
        let mut min_x = first_center.x.clone();
        let mut max_x = first_center.x.clone();
        let mut min_y = first_center.y.clone();
        let mut max_y = first_center.y;
        for &child_idx in &children[1..len] {
            let center = self.nodes[child_idx].bounds.center();
            if center.x < min_x {
                min_x = center.x.clone();
            }
            if center.x > max_x {
                max_x = center.x.clone();
            }
            if center.y < min_y {
                min_y = center.y.clone();
            }
            if center.y > max_y {
                max_y = center.y.clone();
            }
        }
        let split_on_x = (max_x - min_x) > (max_y - min_y);

        // Seed each group with the extreme child on that axis.
        let mut min_pos = 0;
        let mut max_pos = 0;
        for pos in 1..len {
            let center = self.nodes[children[pos]].bounds.center();
            let min_center = self.nodes[children[min_pos]].bounds.center();
            let max_center = self.nodes[children[max_pos]].bounds.center();
            if split_on_x {
                if center.x < min_center.x {
                    min_pos = pos;
                }
                if center.x > max_center.x {
                    max_pos = pos;
                }
            } else {
                if center.y < min_center.y {
                    min_pos = pos;
                }
                if center.y > max_center.y {
                    max_pos = pos;
                }
            }
        }
        if min_pos == max_pos {
            max_pos = (min_pos + 1) % len;
        }

        let mut group_a = [0usize; MAX_CHILDREN + 1];
        let mut group_b = [0usize; MAX_CHILDREN + 1];
        group_a[0] = children[min_pos];
        let mut len_a = 1;
        group_b[0] = children[max_pos];
        let mut len_b = 1;
        let mut bounds_a = self.nodes[group_a[0]].bounds.clone();
        let mut bounds_b = self.nodes[group_b[0]].bounds.clone();

        // Assign each remaining child to the group it expands least.
        let max_group_size = len - 2;
        for (pos, &child_idx) in children[..len].iter().enumerate() {
            if pos == min_pos || pos == max_pos {
                continue;
            }
            let child_bounds = &self.nodes[child_idx].bounds;
            let take_a = if len_a >= max_group_size {
                false
            } else if len_b >= max_group_size {
                true
            } else {
                let growth_a =
                    bounds_a.union(child_bounds).half_perimeter() - bounds_a.half_perimeter();
                let growth_b =
                    bounds_b.union(child_bounds).half_perimeter() - bounds_b.half_perimeter();
                if growth_a == growth_b {
                    len_a <= len_b
                } else {
                    growth_a < growth_b
                }
            };
            if take_a {
                bounds_a = bounds_a.union(child_bounds);
                group_a[len_a] = child_idx;
                len_a += 1;
            } else {
                bounds_b = bounds_b.union(child_bounds);
                group_b[len_b] = child_idx;
                len_b += 1;
            }
        }

        // Reuse `node_idx` for the first group and allocate a sibling for the second.
        if let NodeKind::Internal { children } = &mut self.nodes[node_idx].kind {
            children.indices[..len_a].copy_from_slice(&group_a[..len_a]);
            children.len = len_a as u8;
        }
        self.refresh_node(node_idx);

        let mut sibling_children = NodeChildren::new();
        for &child_idx in &group_b[..len_b] {
            sibling_children.push(child_idx);
        }
        let sibling_idx = self.nodes.len();
        self.nodes.push(Node {
            bounds: Bounds::default(),
            max_order: 0,
            kind: NodeKind::Internal {
                children: sibling_children,
            },
        });
        self.refresh_node(sibling_idx);

        sibling_idx
    }

    /// Creates a new root above two split nodes.
    fn make_root(&mut self, a_idx: usize, b_idx: usize) {
        let (first, second) = if self.nodes[a_idx].max_order <= self.nodes[b_idx].max_order {
            (a_idx, b_idx)
        } else {
            (b_idx, a_idx)
        };
        let bounds = self.nodes[a_idx].bounds.union(&self.nodes[b_idx].bounds);
        let max_order = cmp::max(self.nodes[a_idx].max_order, self.nodes[b_idx].max_order);
        let mut children = NodeChildren::new();
        children.push(first);
        children.push(second);
        let root_idx = self.nodes.len();
        self.nodes.push(Node {
            bounds,
            max_order,
            kind: NodeKind::Internal { children },
        });
        self.root = Some(root_idx);
    }
}

impl<U> Default for BoundsTree<U>
where
    U: Clone + Debug + Default + PartialEq,
{
    fn default() -> Self {
        BoundsTree {
            nodes: Vec::new(),
            root: None,
            max_leaf: None,
            insert_path: Vec::new(),
            search_stack: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Bounds, Point, Size};
    use rand::{Rng, SeedableRng};

    fn tree_depth<U>(nodes: &[Node<U>], idx: usize) -> usize
    where
        U: Clone + Debug + Default + PartialEq,
    {
        match &nodes[idx].kind {
            NodeKind::Leaf { .. } => 1,
            NodeKind::Internal { children } => {
                1 + children
                    .as_slice()
                    .iter()
                    .map(|&child_idx| tree_depth(nodes, child_idx))
                    .max()
                    .unwrap_or(0)
            }
        }
    }

    #[test]
    fn column_insertion_stays_balanced() {
        for count in [100usize, 1000, 5000] {
            let mut tree = BoundsTree::<f32>::default();
            for i in 0..count {
                tree.insert(Bounds {
                    origin: Point {
                        x: 0.0,
                        y: i as f32,
                    },
                    size: Size {
                        width: 100.0,
                        height: 1.0,
                    },
                });
            }

            let depth = tree
                .root
                .map(|root| tree_depth(&tree.nodes, root))
                .unwrap_or(0);
            assert!(depth <= 8, "column of {count} inserted at depth {depth}");
        }
    }

    #[test]
    fn overlapping_column_matches_brute_force() {
        let mut tree = BoundsTree::<f32>::default();
        let mut expected: Vec<(Bounds<f32>, u32)> = Vec::new();
        for i in 0..2000u32 {
            let bounds = Bounds {
                origin: Point {
                    x: 0.0,
                    y: i as f32 * 0.5,
                },
                size: Size {
                    width: 100.0,
                    height: 1.0,
                },
            };
            let order = expected
                .iter()
                .filter(|(existing, _)| existing.intersects(&bounds))
                .map(|(_, order)| *order)
                .max()
                .unwrap_or(0)
                + 1;
            assert_eq!(tree.insert(bounds), order);
            expected.push((bounds, order));
        }
    }

    #[test]
    fn test_insert() {
        let mut tree = BoundsTree::<f32>::default();
        let bounds1 = Bounds {
            origin: Point { x: 0.0, y: 0.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };
        let bounds2 = Bounds {
            origin: Point { x: 5.0, y: 5.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };
        let bounds3 = Bounds {
            origin: Point { x: 10.0, y: 10.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };

        // Insert the bounds into the tree and verify the order is correct
        assert_eq!(tree.insert(bounds1), 1);
        assert_eq!(tree.insert(bounds2), 2);
        assert_eq!(tree.insert(bounds3), 3);

        // Insert non-overlapping bounds and verify they can reuse orders
        let bounds4 = Bounds {
            origin: Point { x: 20.0, y: 20.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };
        let bounds5 = Bounds {
            origin: Point { x: 40.0, y: 40.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };
        let bounds6 = Bounds {
            origin: Point { x: 25.0, y: 25.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };
        assert_eq!(tree.insert(bounds4), 1); // bounds4 does not overlap with bounds1, bounds2, or bounds3
        assert_eq!(tree.insert(bounds5), 1); // bounds5 does not overlap with any other bounds
        assert_eq!(tree.insert(bounds6), 2); // bounds6 overlaps with bounds4, so it should have a different order
    }

    #[test]
    fn test_insert_group_reserves_order_range() {
        let mut tree = BoundsTree::<f32>::default();
        let group = Bounds {
            origin: Point { x: 0.0, y: 0.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };
        // The group spans 4 order steps starting at 1.
        assert_eq!(tree.insert_group(group, 4), 1);

        // A later overlapping bound sorts above the group's maximum order (5).
        let overlapping = Bounds {
            origin: Point { x: 5.0, y: 5.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };
        assert_eq!(tree.insert(overlapping), 6);

        // A disjoint bound is unaffected by the group.
        let disjoint = Bounds {
            origin: Point { x: 100.0, y: 100.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };
        assert_eq!(tree.insert(disjoint), 1);
    }

    #[test]
    fn split_propagation_matches_brute_force() {
        for seed in 1..=200u64 {
            let mut tree = BoundsTree::default();
            let mut expected: Vec<(Bounds<f32>, u32)> = Vec::new();
            let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
            let num_bounds = rng.random_range(1..=300);
            for i in 0..num_bounds {
                let min_x: f32 = rng.random_range(-60.0..60.0);
                let min_y: f32 = rng.random_range(-60.0..60.0);
                let width: f32 = rng.random_range(0.0..45.0);
                let height: f32 = rng.random_range(0.0..45.0);
                let bounds = Bounds {
                    origin: Point { x: min_x, y: min_y },
                    size: Size { width, height },
                };
                let expected_ordering = expected
                    .iter()
                    .filter_map(|(existing, order)| existing.intersects(&bounds).then_some(*order))
                    .max()
                    .unwrap_or(0)
                    + 1;
                let actual = tree.insert(bounds);
                if actual != expected_ordering {
                    panic!(
                        "seed={seed} i={i} expected={expected_ordering} actual={actual} bounds={bounds:?}"
                    );
                }
                expected.push((bounds, expected_ordering));
            }
        }
    }

    #[test]
    fn disjoint_global_max_forces_tree_search() {
        let mut tree = BoundsTree::<f32>::default();
        let mut expected: Vec<(Bounds<f32>, u32)> = Vec::new();

        // Interleave two far-apart columns of overlapping strips. Each query
        // overlaps its own column but not the most recently inserted strip in
        // the other column, so the max-leaf fast path never applies and the
        // lookup must traverse split ancestors.
        for i in 0..400 {
            let bounds = Bounds {
                origin: Point {
                    x: if i % 2 == 0 { 0.0 } else { 1000.0 },
                    y: (i / 2) as f32 * 2.0,
                },
                size: Size {
                    width: 10.0,
                    height: 5.0,
                },
            };
            let expected_ordering = expected
                .iter()
                .filter_map(|(existing, order)| existing.intersects(&bounds).then_some(*order))
                .max()
                .unwrap_or(0)
                + 1;
            assert_eq!(tree.insert(bounds), expected_ordering);
            expected.push((bounds, expected_ordering));
        }
    }

    #[test]
    fn test_random_iterations() {
        let max_bounds = 100;
        for seed in 1..=1000 {
            // let seed = 44;
            let mut tree = BoundsTree::default();
            let mut rng = rand::rngs::StdRng::seed_from_u64(seed as u64);
            let mut expected_quads: Vec<(Bounds<f32>, u32)> = Vec::new();

            // Insert a random number of random AABBs into the tree.
            let num_bounds = rng.random_range(1..=max_bounds);
            for _ in 0..num_bounds {
                let min_x: f32 = rng.random_range(-100.0..100.0);
                let min_y: f32 = rng.random_range(-100.0..100.0);
                let width: f32 = rng.random_range(0.0..50.0);
                let height: f32 = rng.random_range(0.0..50.0);
                let bounds = Bounds {
                    origin: Point { x: min_x, y: min_y },
                    size: Size { width, height },
                };

                let expected_ordering = expected_quads
                    .iter()
                    .filter_map(|quad| quad.0.intersects(&bounds).then_some(quad.1))
                    .max()
                    .unwrap_or(0)
                    + 1;
                expected_quads.push((bounds, expected_ordering));

                // Insert the AABB into the tree and collect intersections.
                let actual_ordering = tree.insert(bounds);
                assert_eq!(actual_ordering, expected_ordering);
            }
        }
    }
}
