//! An implementation of red-black trees, which are self balancing BSTs.
//!
//! based on wikipedia's article.

use core::{cmp::Ordering, fmt::Debug, ptr::NonNull};

use crate::collections::QueryFor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Represents the color of an RBTree's Node to follow 2 of the requirements:
///
/// - all null nodes are considered black.
/// - A red node cannot have a red child
/// - Every path from a given node to any of its leaf nodes goes through the same number of black nodes.
enum Color {
    Red = 0,
    Black = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Represents whether the current node is a left child or a right child of it's parent.
pub enum Direction {
    /// The current node is less than its parent, so it is a left child.
    Left,
    /// The current node is greater than its parent, so it is a right child.
    Right,
}

impl Direction {
    /// Reverse this direction
    #[inline(always)]
    pub const fn opposite(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ColoredPtrTag(usize);
impl ColoredPtrTag {
    #[inline(always)]
    pub fn set_color(&mut self, color: Color) {
        let v = color as usize;
        self.0 = (self.0 & !1) | v;
    }

    #[inline(always)]
    pub const fn color(&self) -> Color {
        if self.0 & 1 == 1 {
            Color::Black
        } else {
            Color::Red
        }
    }

    #[inline(always)]
    pub const fn ptr_to<T>(&self) -> Option<NonNull<T>> {
        NonNull::new((self.0 & !1) as *mut T)
    }

    #[inline(always)]
    pub fn set_ptr_to<T>(&mut self, ptr: Option<NonNull<T>>) {
        let color = self.0 & 1;
        self.0 = match ptr {
            Some(p) => p.as_ptr() as usize | color,
            None => 0 | color,
        };
    }
}

/// A node within the tree can be a subnode or a root node.
///
/// left of the node are nodes with [`Ordering::Less`] keys than it's key, right of the node are ones with [`Ordering::Greater`], both sides could contain [`Ordering::Equal`] keys.
///
/// if either left or right or None they are called null nodes instead of falling out of equation.
pub struct Node<T> {
    value: T,
    colored_parent: ColoredPtrTag,
    pub(super) left: Option<NonNull<Self>>,
    pub(super) right: Option<NonNull<Self>>,
}

impl<T: Debug> Debug for Node<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Node")
            .field("parent", &self.parent())
            .field("color", &self.color())
            .field("value", &self.value)
            .field("left", &self.left)
            .field("right", &self.right)
            .finish()
    }
}

impl<T> Node<T> {
    /// Returns a new node with the given value.
    #[inline(always)]
    pub const fn new(value: T) -> Self {
        Self {
            value,
            colored_parent: ColoredPtrTag(0),
            left: None,
            right: None,
        }
    }

    /// Returns a reference to the value of this node.
    #[inline(always)]
    pub const fn value(&self) -> &T {
        &self.value
    }

    /// Returns a mutable reference to the value of this node.
    ///
    /// # Safety
    ///
    /// If the value was modified, the tree must be rebalanced.
    #[inline(always)]
    pub unsafe fn value_mut(&mut self) -> &mut T {
        &mut self.value
    }
}

impl<T> Node<T> {
    #[inline(always)]
    pub const fn parent(&self) -> Option<NonNull<Self>> {
        self.colored_parent.ptr_to::<Self>()
    }

    #[inline(always)]
    pub const fn color(&self) -> Color {
        self.colored_parent.color()
    }

    #[inline(always)]
    pub fn set_parent(&mut self, parent: Option<NonNull<Self>>) {
        self.colored_parent.set_ptr_to(parent);
    }

    #[inline(always)]
    pub fn set_color(&mut self, color: Color) {
        self.colored_parent.set_color(color);
    }

    #[inline(always)]
    pub fn parent_ref<'s>(&'s self) -> Option<&'s Self> {
        self.parent().map(|p| unsafe { p.as_ref() })
    }

    #[inline(always)]
    pub fn direction(&self) -> Option<Direction> {
        self.parent_ref().map(|p| {
            if p.left.is_some_and(|pl| core::ptr::eq(pl.as_ptr(), self)) {
                Direction::Left
            } else {
                Direction::Right
            }
        })
    }

    /// Returns the child at a given direction.
    #[inline(always)]
    pub const fn child(&self, dir: Direction) -> Option<NonNull<Self>> {
        match dir {
            Direction::Left => self.left,
            Direction::Right => self.right,
        }
    }

    #[inline(always)]
    pub const fn child_mut(&mut self, dir: Direction) -> &mut Option<NonNull<Self>> {
        match dir {
            Direction::Left => &mut self.left,
            Direction::Right => &mut self.right,
        }
    }
}

#[derive(Debug)]
pub struct RawRBTree<T> {
    root: Option<NonNull<Node<T>>>,
}

impl<T: core::fmt::Debug> RawRBTree<T> {
    fn balance_check_recursive(
        &self,
        node: Option<NonNull<Node<T>>>,
    ) -> Result<(), alloc::string::String> {
        if let Some(node) = node {
            let node_ref = unsafe { node.as_ref() };
            let node_is_red = node_ref.color() == Color::Red;
            let left = node_ref.left;
            let right = node_ref.right;
            unsafe {
                match (left, right) {
                    (Some(c), None) | (None, Some(c)) => {
                        if c.as_ref().color() != Color::Red {
                            return Err(alloc::format!(
                                "Node: {:?} has one child and it isn't red: {:?}",
                                node_ref,
                                c.as_ref()
                            ));
                        }
                    }
                    _ => {}
                }

                if node_is_red
                    && let Some(left) = left
                    && left.as_ref().color() != Color::Black
                {
                    return Err(alloc::format!(
                        "Red node: {:?} has a red left child: {:?}",
                        node_ref,
                        left.as_ref()
                    ));
                }

                if node_is_red
                    && let Some(right) = right
                    && right.as_ref().color() != Color::Black
                {
                    return Err(alloc::format!(
                        "Red node: {:?} has a red right child: {:?}",
                        node_ref,
                        left.as_ref()
                    ));
                }
            }
            self.balance_check_recursive(left)?;
            self.balance_check_recursive(right)?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn balance_check(&self) -> Result<(), alloc::string::String> {
        self.balance_check_recursive(self.root)
    }
}

impl<T> RawRBTree<T> {
    const SIZE_OF_NODE: usize = size_of::<Node<T>>();
    pub const fn new() -> Self {
        Self { root: None }
    }

    pub const fn root(&self) -> Option<NonNull<Node<T>>> {
        self.root
    }

    /// Clears the tree, removing all nodes, but it doesn't free the memory or drop the values.
    pub const fn clear(&mut self) {
        self.root = None;
    }

    /// Rotates a subtree in a given direction.
    ///
    /// returns the new parent of `sub`, `sub` will have one of it's nodes rotated according to `dir` to be above it.
    ///
    /// panicks if the child to rotate (left on a right rotation, right on a left rotation) is not there.
    fn rotate_subtree(&mut self, mut sub: NonNull<Node<T>>, dir: Direction) -> NonNull<Node<T>> {
        let sub_mut = unsafe { sub.as_mut() };

        let sub_dir = sub_mut.direction();
        let old_root = sub_mut.parent();

        // picking a new root and stealing it's child
        //
        // ex. in a left rotation, we want to rotate the right child to the left, making it our root
        // and it's left child is going to be our new right child (more than us), and we will be it's left child (less than it).
        let mut new_root = sub_mut
            .child(dir.opposite())
            .expect("A child must exist at the opposite direction of the rotation");
        let new_root_mut = unsafe { new_root.as_mut() };

        // setting the child
        let new_child = new_root_mut.child(dir);
        *sub_mut.child_mut(dir.opposite()) = new_child;
        if let Some(mut new) = new_child {
            unsafe { new.as_mut().set_parent(Some(sub)) };
        }
        // replacing the child we toke with ourselves
        *new_root_mut.child_mut(dir) = Some(sub);

        // making ourselves a child of new_root and giving it up for adoption by the old_root
        new_root_mut.set_parent(old_root);
        sub_mut.set_parent(Some(new_root));

        if let Some(mut old) = old_root {
            // there is a root which was beyond this one
            let sub_dir = sub_dir.expect("Direction should be Some because parent is");
            unsafe { *old.as_mut().child_mut(sub_dir) = Some(new_root) };
        } else {
            // the new root is root
            self.root = Some(new_root);
        }

        new_root
    }

    /// Inserts an orphan (parentless) `node` at the `direction` child of the given `parent_node`.
    ///
    /// Safety:
    /// - parent_node child at `direction` must be null and the correct location for `node` (according to key), and if parent_node is none, then node must be expected to replace the root node.
    /// - this function takes raw pointers, these must be valid and not in use anywhere else, and `parent_node` must belong to the tree.
    pub unsafe fn insert_at_node(
        &mut self,
        mut node: NonNull<Node<T>>,
        parent_node: Option<NonNull<Node<T>>>,
        direction: Direction,
    ) {
        unsafe {
            let node_mut = node.as_mut();
            node_mut.set_parent(parent_node);
            node_mut.set_color(Color::Red);
            node_mut.left = None;
            node_mut.right = None;

            if let Some(mut p) = parent_node {
                let child_slot = p.as_mut().child_mut(direction);
                let old_child = *child_slot;

                *child_slot = Some(node);
                // Taking the place of an old node.
                if let Some(old_child) = old_child {
                    let old_child_ref = old_child.as_ref();
                    node_mut.set_color(old_child_ref.color());

                    if let Some(mut left) = old_child_ref.left {
                        node_mut.left = Some(left);
                        left.as_mut().set_parent(Some(node));
                    }
                    if let Some(mut right) = old_child_ref.right {
                        node_mut.right = Some(right);
                        right.as_mut().set_parent(Some(node));
                    }
                    return;
                }
            } else {
                self.root = Some(node);
                return;
            }
        }

        let mut current = Some(node);
        // Rebalance the tree to uphold to the 5 commandments
        //
        // Ran with the assumption that node is red.
        while let Some(node) = current
            && let Some(mut parent) = unsafe { node.as_ref().parent() }
        {
            let node_dir = unsafe {
                node.as_ref()
                    .direction()
                    .expect("parent is some, and so is direction")
            };
            let parent_ref = unsafe { parent.as_ref() };
            // According to the safety warning above, nothing more to worry about if parent is already black.
            if parent_ref.color() == Color::Black {
                return;
            }
            // Otherwise, parent is red, god is racist, deploy all angles, we must use force.
            // (the idea is that we keep enforcing the red doesn't have red children rule here)

            // grandparent is always black if parent is red.
            let Some(mut grandparent) = parent_ref.parent() else {
                // No grandparents to complain against such a decision, parent must be eve (root) (black height will increase by 1 here).
                unsafe { parent.as_mut().set_color(Color::Black) };
                return;
            };

            let grandparent_ref = unsafe { grandparent.as_ref() };
            let parent_dir = parent_ref
                .direction()
                .expect("Should be some because we got a grandparent");

            // I met God, she is black, matriarchal
            // and she doesn't like the reds very much...
            let aunt_dir = parent_dir.opposite();
            let aunt = grandparent_ref.child(aunt_dir);

            // If aunt is black and parent red (grandparent is also black).
            // remember null nodes are considered black
            //
            // the ultimate goal is to rotate the parent node to the position of grandparent (move up the red).
            if aunt.is_none_or(|aunt| unsafe { aunt.as_ref().color() == Color::Black }) {
                // by extension node_dir == invert of parent_dir
                if aunt_dir == node_dir {
                    // parent is red, but sibling is black
                    // by rotating the tree in the (red)parent direction we make the `node` become the parent (and inverted it's direction).
                    // (because it is against the parent direction).
                    //
                    // the puropse of this rotation is to prevent the red node from becoming a child of the future to be red grandparent.
                    parent = self.rotate_subtree(parent, parent_dir);
                }

                // By rotating grandparent's tree in the aunt's direction (opposite of parent),
                //
                // we have made (red)parent the grandparent.
                //
                // the parent's `aunt_dir` child has becomes the grandparent's `parent_dir` child,
                // as long as it isn't red this is fine which should be true if it isn't `node` (we solve the issue by the rotation above).
                self.rotate_subtree(grandparent, aunt_dir);
                unsafe { parent.as_mut().set_color(Color::Black) };
                unsafe { grandparent.as_mut().set_color(Color::Red) };
                return;
            }

            let mut aunt = aunt.unwrap();

            // Both parent and aunt are red.
            // (grandparent must be black).
            //
            // so we can just invert the colors, this won't affect black height as the number of black nodes through every path won't change (because going through grandparent who was black will now yield red).
            //
            // if grandparent has a red parent it may viloate the rules and therefore we gotta go all the way up
            unsafe { parent.as_mut().set_color(Color::Black) };
            unsafe { aunt.as_mut().set_color(Color::Black) };
            unsafe { grandparent.as_mut().set_color(Color::Red) };

            current = Some(grandparent);
        }

        // At worse case the tree should now be balanced.
        // but it's black height would still increase by 1, which is always good (see line 193)
    }

    #[inline(always)]
    unsafe fn rebalance_childless_node(
        &mut self,
        mut parent: NonNull<Node<T>>,
        node_direction: Direction,
    ) {
        // sibling must be black
        // this will return true if the sibling's tree was rebalanced, false otherwise
        unsafe fn nibling_rebalance_attempt<T>(
            tree: &mut RawRBTree<T>,
            mut parent: NonNull<Node<T>>,
            mut sibling: NonNull<Node<T>>,
            direction: Direction,
        ) -> bool {
            let mut distant_nibling = unsafe { sibling.as_ref().child(direction.opposite()) };
            let close_nibling = unsafe { sibling.as_ref().child(direction) };

            let mut handled = false;
            // Case 5
            if let Some(mut close_n) = close_nibling
                && unsafe { close_n.as_ref().color() == Color::Red }
            {
                // After rotation at the sibling's direction, the close nibling becomes our sibling, and the sibling's parent
                //
                // it is red, and the sibling is black, that means it may only have black children
                tree.rotate_subtree(sibling, direction.opposite());

                // if we swap colors of sibling and our new sibling, the black height wouldn't change now
                // and we have fulfilled the requirement for Case 6
                unsafe {
                    sibling.as_mut().set_color(Color::Red);
                    close_n.as_mut().set_color(Color::Black);
                }
                distant_nibling = Some(sibling);
                sibling = close_n;
                handled = true;
            }

            // Case 6
            //
            // For this case we need a sibling that is black with a red distant nibling.
            if let Some(mut distant_n) = distant_nibling
                && unsafe { distant_n.as_ref().color() == Color::Red }
            {
                // After this rotation, sibling(black) will become our grandparent
                tree.rotate_subtree(parent, direction);

                unsafe {
                    // Exchange colors with sibling and parent, then make distant nibling black.
                    //
                    // This does nothing if parent was black
                    sibling.as_mut().set_color(parent.as_ref().color());
                    // If parent was red, we make it black because sibling is now red.
                    parent.as_mut().set_color(Color::Black);
                    // distant nibling was red, if parent was red, sibling is now red, so we make it black
                    // if parent was black, we became 1 black height less because of the rotation so we have to color this black.
                    distant_n.as_mut().set_color(Color::Black);
                }
                handled = true;
            }

            handled
        }

        unsafe { *parent.as_mut().child_mut(node_direction) = None };

        let mut direction = node_direction;
        loop {
            let sibling_dir = direction.opposite();
            let sibling_ptr = unsafe { parent.as_mut().child(sibling_dir) };
            let Some(mut sibling) = sibling_ptr else {
                break;
            };

            // attempts to make the sibling black to call `nibling_rebalance_attempt`
            if unsafe { sibling.as_ref().color() == Color::Red } {
                // this means the parent, distant nibling/close nibling are black
                //
                // rotating the tree here makes the sibling our grandparent
                self.rotate_subtree(parent, direction);

                unsafe {
                    // if we swap colors of sibling and parent,
                    // the node will have a red parent, and another black sibling (close nibling)
                    sibling.as_mut().set_color(Color::Black);
                    parent.as_mut().set_color(Color::Red);
                }

                // sibling is now the grandparent, use the new sibling
                let mut sibling = unsafe { parent.as_mut().child(sibling_dir) }
                    .expect("parent must have a sibling after rotation");

                if !unsafe { nibling_rebalance_attempt(self, parent, sibling, direction) } {
                    unsafe {
                        // this means we had no sibling's children (both of them are considered black).
                        sibling.as_mut().set_color(Color::Red);
                        parent.as_mut().set_color(Color::Black);
                    }
                };

                break;
            }

            // Sibling is already black.
            if !unsafe { nibling_rebalance_attempt(self, parent, sibling, direction) } {
                // Case 4:
                //
                // Swap sibling and parent colors
                //
                // If this path was reached, it means that the sibling and it's children are all black.
                // meaning that swapping colors will not affect the black height of the tree from the sibling's perspective.
                //
                // however it will add one more black height from the node's perspective, which was just removed.
                if unsafe { parent.as_ref().color() } == Color::Red {
                    unsafe {
                        sibling.as_mut().set_color(Color::Red);
                        parent.as_mut().set_color(Color::Black);
                    }
                    break;
                }

                // Both parent and sibling are black.
                //
                // continue trying to decrease black height.
                //
                // that is if this path keeps getting caught.
                // Case 2:
                unsafe {
                    sibling.as_mut().set_color(Color::Red);
                    let node = parent;
                    if let Some(n_parent) = node.as_ref().parent() {
                        parent = n_parent;
                        direction = node
                            .as_ref()
                            .direction()
                            .expect("Parent exists and so does direction");
                        continue;
                    }
                }
            }

            // Once the above function is executed successfully, we can return
            break;
        }
    }

    /// Removes a node in place.
    ///
    /// After this the node is safe to deallocate (won't be deallocated by this).
    /// The node would still be safe to read, but its value may have changed.
    ///
    /// # Safety:
    /// - `node` must be valid and belong to the tree.
    pub unsafe fn remove_node(&mut self, mut node: NonNull<Node<T>>) {
        let node_ref = unsafe { node.as_ref() };

        let parent = node_ref.parent();
        let parent_child_slot = node_ref
            .direction()
            .map(|d| {
                unsafe {
                    parent
                        .expect("Has direction so should have parent")
                        .as_mut()
                }
                .child_mut(d)
            })
            .unwrap_or(&mut self.root);

        let left = node_ref.left;
        let right = node_ref.right;
        match (left, right) {
            // Node has one child, it is well-defined that this child must be red and the node must be black in that case
            // so we can safely swap the node and child, preserving the black height of the tree.
            (Some(mut child), None) | (None, Some(mut child)) => {
                let child_mut = unsafe { child.as_mut() };
                debug_assert_eq!(
                    child_mut.color(),
                    Color::Red,
                    "Any single child must be red"
                );
                debug_assert_eq!(
                    node_ref.color(),
                    Color::Black,
                    "Any parent of a red node must be black"
                );

                child_mut.set_color(Color::Black);
                child_mut.set_parent(parent);

                *parent_child_slot = Some(child);
            }
            (Some(mut left), Some(mut right)) => {
                // we want to get the leftmost (in-order successor) node of the right subtree (to replace with the node to remove)
                let mut successor = right;
                let mut successor_parent = node;

                while let Some(left) = unsafe { successor.as_ref() }.left {
                    successor_parent = successor;
                    successor = left;
                }

                // wikipedia says to just swap values, however this function guarantees that the given pointer is truly removed from the tree, and is still valid.

                let successor_mut = unsafe { successor.as_mut() };
                let node_mut = unsafe { node.as_mut() };

                // instead of swapping just value swap what is inside the pointers first.
                core::mem::swap(successor_mut, node_mut);

                // swap the pointers
                // too much spaghetti of logic.
                //
                // TODO: maybe renaming successor and node to each other would be cleaner? maybe there is a better way to do this?
                *parent_child_slot = Some(successor);

                // update parent pointers of children.

                unsafe {
                    left.as_mut().set_parent(Some(successor));
                };

                // this is only possible if the successor is the right child of node, or more correctly was
                if successor_parent == node {
                    // parent -> node -> successor
                    // we made parent -> successor
                    // now we make successor -> node
                    //
                    // otherwise we will be making node -> node if we don't handle this case specially
                    successor_mut.right = Some(node);
                    node_mut.set_parent(Some(successor));
                    // we don't set the parent of the right child here because it is == successor
                } else {
                    node_mut.set_parent(Some(successor_parent));
                    unsafe {
                        successor_parent.as_mut().left = Some(node);
                        right.as_mut().set_parent(Some(successor));
                    };
                }

                // if successor had a right child, update its parent pointer
                if let Some(mut c) = node_mut.right {
                    unsafe { c.as_mut().set_parent(Some(node)) };
                }
                // successor cannot possibly have a left child, as it is the in-order successor of node

                // now swap the data just as wikipedia says.
                core::mem::swap(&mut successor_mut.value, &mut node_mut.value);

                // the node pointer, is actually the successor's pointer now.
                // we are still moving the node pointer, upholding our promises.
                return unsafe { self.remove_node(node) };
            }
            (None, None) if let Some(parent) = parent => {
                if node_ref.color() == Color::Red {
                    *parent_child_slot = None;
                } else {
                    unsafe {
                        self.rebalance_childless_node(
                            parent,
                            node_ref
                                .direction()
                                .expect("Node has parent so should have direction"),
                        )
                    };
                }
            }
            (None, None) => {
                // Node is root with no children
                debug_assert_eq!(self.root, Some(node), "node to remove should be root");
                self.root = None;
            }
        }
    }
}

impl<T> RawRBTree<T> {
    /// Given a search key, returns a `(node, parent, direction)` tuple.
    pub fn search<Q: QueryFor<T>>(
        &self,
        key: &Q,
    ) -> (
        Option<NonNull<Node<T>>>,
        Option<NonNull<Node<T>>>,
        Direction,
    ) {
        let mut node = self.root;
        let mut parent = None;
        let mut direction = Direction::Left;

        while let Some(n) = node {
            let cmp = key.compare(&unsafe { n.as_ref() }.value);
            if cmp == Ordering::Equal {
                return (Some(n), parent, direction);
            }

            if cmp == Ordering::Greater {
                direction = Direction::Right;
            } else {
                direction = Direction::Left;
            }
            parent = Some(n);
            node = unsafe { n.as_ref() }.child(direction);
        }

        (None, parent, direction)
    }
}
