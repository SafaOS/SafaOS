use crate::collections::RBTree;
use alloc::vec;
use alloc::vec::Vec;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Spec {
    magic: usize,
}

#[test]
pub fn a_insert_rbtree() {
    let mut tree = RBTree::<usize, Spec>::new();
    tree.try_insert(6, Spec { magic: 42 })
        .expect("Allocation error trying to insert into RBTree");
    assert_eq!(tree.len(), 1);
}

#[test]
pub fn b_insert_and_search() {
    let mut tree = RBTree::<usize, Spec>::new();
    tree.try_insert(6, Spec { magic: 556 })
        .expect("Allocation error trying to insert into RBTree");
    assert_eq!(tree.get(&6), Some(&Spec { magic: 556 }));
    assert_eq!(tree.remove(&6), Some((6, Spec { magic: 556 })));
    assert_eq!(tree.get(&6), None);
    assert_eq!(tree.len(), 0);
}

#[test]
pub fn c_big_tree() {
    const KEYS: [usize; 20] = [
        1, 3, 2, 5, 4, 6, 9, 8, 7, 10, 12, 13, 17, 11, 15, 16, 14, 18, 19, 42,
    ];

    let magic_base = 42;
    let mut tree = RBTree::<usize, Spec>::new();
    for &key in &KEYS {
        tree.try_insert(
            key,
            Spec {
                magic: magic_base + key,
            },
        )
        .expect("Allocation error trying to insert into RBTree");
    }
    assert_eq!(tree.len(), KEYS.len());

    for &key in &KEYS {
        assert_eq!(
            tree.get(&key),
            Some(&Spec {
                magic: magic_base + key
            })
        );
    }

    tree.raw.balance_check().expect("Tree balance check failed");

    for &key in &KEYS {
        assert_eq!(
            tree.try_insert(
                key,
                Spec {
                    magic: magic_base + key + 1,
                },
            )
            .expect("Allocation error trying to insert into RBTree"),
            Some(Spec {
                magic: magic_base + key,
            }),
            "Attempt to overwrite existing key test pass failure"
        );
    }
    assert_eq!(tree.len(), KEYS.len());

    for i in 0..KEYS.len() {
        let key = KEYS[i];
        assert_eq!(
            tree.remove(&key),
            Some((
                key,
                Spec {
                    magic: magic_base + key + 1
                }
            )),
        );
        tree.raw.balance_check().expect("Tree balance check failed");

        let Some(next_key) = KEYS.get(i + 1) else {
            continue;
        };
        assert_eq!(
            tree.get(&next_key),
            Some(&Spec {
                magic: magic_base + next_key + 1
            }),
        );
    }
    assert_eq!(tree.len(), 0);
}

// ======= AI Generated =======
use crate::collections::rbtree::linked_rbtree::LinkedRBTree;

use core::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

fn xorshift(seed: &mut u32) -> u32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    *seed
}

fn ascending(tree: &LinkedRBTree<i32, i32>) -> Vec<i32> {
    let mut out = Vec::new();
    let mut cur = tree.front_cursor();
    while let Some((k, _)) = cur.key_value() {
        out.push(*k);
        cur.move_next();
    }
    out
}

// ---- construction & basic accessors ------------------------------

#[test]
fn d_a_new_tree_is_empty() {
    let tree: LinkedRBTree<i32, i32> = LinkedRBTree::new();
    assert_eq!(tree.len(), 0);
    assert_eq!(tree.get(&0), None);
    assert!(tree.front_cursor().key_value().is_none());
    assert!(tree.back_cursor().key_value().is_none());
}

#[test]
fn d_b_insert_then_get() {
    let mut tree = LinkedRBTree::new();
    assert_eq!(tree.try_insert(1, "one").unwrap(), None);
    assert_eq!(tree.len(), 1);
    assert_eq!(tree.get(&1), Some(&"one"));
}

#[test]
fn d_c_reinsert_replaces_value_keeps_len() {
    let mut tree = LinkedRBTree::new();
    tree.try_insert(1, "one").unwrap();
    let old = tree.try_insert(1, "uno").unwrap();
    assert_eq!(old, Some("one"));
    assert_eq!(tree.len(), 1);
    assert_eq!(tree.get(&1), Some(&"uno"));
}

#[test]
fn d_d_get_mut_mutates_in_place() {
    let mut tree = LinkedRBTree::new();
    tree.try_insert(1, 10).unwrap();
    *tree.get_mut(&1).unwrap() += 5;
    assert_eq!(tree.get(&1), Some(&15));
}

// ---- removal -------------------------------------------------------

#[test]
fn d_e_remove_existing_shrinks_len() {
    let mut tree = LinkedRBTree::new();
    tree.try_insert(1, "one").unwrap();
    assert_eq!(tree.remove(&1), Some((1, "one")));
    assert_eq!(tree.len(), 0);
    assert_eq!(tree.get(&1), None);
}

#[test]
fn d_f_remove_missing_is_none() {
    let mut tree: LinkedRBTree<i32, i32> = LinkedRBTree::new();
    tree.try_insert(1, 1).unwrap();
    assert_eq!(tree.remove(&99), None);
    assert_eq!(tree.len(), 1);
}

// ---- sorted-order invariant ----------------------------------------

#[test]
fn d_g_sorted_order_ascending_inserts() {
    let mut tree = LinkedRBTree::new();
    for k in 0..20 {
        tree.try_insert(k, k).unwrap();
    }
    assert_eq!(ascending(&tree), (0..20).collect::<Vec<_>>());
}

#[test]
fn d_h_sorted_order_descending_inserts() {
    let mut tree = LinkedRBTree::new();
    for k in (0..20).rev() {
        tree.try_insert(k, k).unwrap();
    }
    assert_eq!(ascending(&tree), (0..20).collect::<Vec<_>>());
}

#[test]
fn d_i_sorted_order_shuffled_inserts() {
    let mut keys: Vec<i32> = (0..200).collect();
    let mut seed = 0xC0FFEEu32;
    for i in (1..keys.len()).rev() {
        let j = (xorshift(&mut seed) as usize) % (i + 1);
        keys.swap(i, j);
    }

    let mut tree = LinkedRBTree::new();
    for &k in &keys {
        tree.try_insert(k, k).unwrap();
    }
    assert_eq!(ascending(&tree), (0..200).collect::<Vec<_>>());
}

#[test]
fn d_i_shuffled_insert_reports_first_orphan() {
    let mut keys: Vec<i32> = (0..200).collect();
    let mut seed = 0xC0FFEEu32;
    for i in (1..keys.len()).rev() {
        let j = (xorshift(&mut seed) as usize) % (i + 1);
        keys.swap(i, j);
    }

    let mut tree = LinkedRBTree::new();
    for (n, &k) in keys.iter().enumerate() {
        tree.try_insert(k, k).unwrap();
        // Check invariant after every single insert, not just at the end.
        let seen = ascending(&tree);
        if seen.len() != n + 1 {
            panic!(
                "orphaned after inserting key {k} (insert #{n}); \
                 prefix so far: {:?}; list has {} of {} nodes",
                &keys[..=n],
                seen.len(),
                n + 1
            );
        }
    }
}

// ---- linked-list relinking around removal --------------------------

#[test]
fn d_j_remove_head_advances_head() {
    let mut tree = LinkedRBTree::new();
    for k in 0..5 {
        tree.try_insert(k, k).unwrap();
    }
    tree.remove(&0);
    assert_eq!(tree.front_cursor().key_value(), Some((&1, &1)));
    assert_eq!(ascending(&tree), vec![1, 2, 3, 4]);
}

#[test]
fn d_k_remove_tail_retreats_tail() {
    let mut tree = LinkedRBTree::new();
    for k in 0..5 {
        tree.try_insert(k, k).unwrap();
    }
    tree.remove(&4);
    assert_eq!(tree.back_cursor().key_value(), Some((&3, &3)));
    assert_eq!(ascending(&tree), vec![0, 1, 2, 3]);
}

#[test]
fn d_l_remove_middle_relinks_neighbors() {
    let mut tree = LinkedRBTree::new();
    for k in 0..5 {
        tree.try_insert(k, k).unwrap();
    }
    tree.remove(&2);
    assert_eq!(ascending(&tree), vec![0, 1, 3, 4]);

    let cur = tree.cursor_to(&1);
    assert_eq!(cur.peek_next(), Some((&3, &3)));
    let cur = tree.cursor_to(&3);
    assert_eq!(cur.peek_prev(), Some((&1, &1)));
}

#[test]
fn d_m_remove_last_node_clears_ends() {
    let mut tree = LinkedRBTree::new();
    tree.try_insert(1, 1).unwrap();
    tree.remove(&1);
    assert!(tree.front_cursor().key_value().is_none());
    assert!(tree.back_cursor().key_value().is_none());
    assert_eq!(tree.len(), 0);
}

// ---- clear -----------------------------------------------------------

#[test]
fn d_n_clear_empties_tree_and_ends() {
    let mut tree = LinkedRBTree::new();
    for k in 0..10 {
        tree.try_insert(k, k).unwrap();
    }
    tree.clear();
    assert_eq!(tree.len(), 0);
    assert!(tree.front_cursor().key_value().is_none());
    assert!(tree.back_cursor().key_value().is_none());
    assert_eq!(tree.get(&3), None);
}

// ---- cursors -----------------------------------------------------------

#[test]
fn d_o_front_and_back_cursor_match_ends() {
    let mut tree = LinkedRBTree::new();
    for k in 0..5 {
        tree.try_insert(k, k * 10).unwrap();
    }
    assert_eq!(tree.front_cursor().key_value(), Some((&0, &0)));
    assert_eq!(tree.back_cursor().key_value(), Some((&4, &40)));
}

#[test]
fn d_p_cursor_to_hits_existing_key() {
    let mut tree = LinkedRBTree::new();
    for k in 0..5 {
        tree.try_insert(k, k).unwrap();
    }
    assert_eq!(tree.cursor_to(&3).key_value(), Some((&3, &3)));
}

#[test]
fn d_q_cursor_to_missing_key_is_ghost() {
    let mut tree = LinkedRBTree::new();
    for k in 0..5 {
        tree.try_insert(k, k).unwrap();
    }
    assert!(tree.cursor_to(&99).key_value().is_none());
}

#[test]
fn d_r_move_next_past_tail_wraps_to_head() {
    let mut tree = LinkedRBTree::new();
    for k in 0..3 {
        tree.try_insert(k, k).unwrap();
    }
    let mut cur = tree.back_cursor();
    cur.move_next(); // ghost (past tail)
    assert!(cur.key_value().is_none());
    cur.move_next(); // wraps to head
    assert_eq!(cur.key_value(), Some((&0, &0)));
}

#[test]
fn d_s_move_prev_past_head_wraps_to_tail() {
    let mut tree = LinkedRBTree::new();
    for k in 0..3 {
        tree.try_insert(k, k).unwrap();
    }
    let mut cur = tree.front_cursor();
    cur.move_prev(); // ghost (before head)
    assert!(cur.key_value().is_none());
    cur.move_prev(); // wraps to tail
    assert_eq!(cur.key_value(), Some((&2, &2)));
}

#[test]
fn d_t_ghost_wrap_is_symmetric_round_trip() {
    let mut tree = LinkedRBTree::new();
    for k in 0..4 {
        tree.try_insert(k, k).unwrap();
    }
    let mut cur = tree.front_cursor();
    for _ in 0..4 {
        cur.move_next();
    }
    cur.move_next(); // ghost -> head
    assert_eq!(cur.key_value(), Some((&0, &0)));

    let mut cur = tree.back_cursor();
    for _ in 0..4 {
        cur.move_prev();
    }
    cur.move_prev(); // ghost -> tail
    assert_eq!(cur.key_value(), Some((&3, &3)));
}

#[test]
fn d_u_peek_prev_next_dont_move_cursor() {
    let mut tree = LinkedRBTree::new();
    for k in 0..3 {
        tree.try_insert(k, k).unwrap();
    }
    let cur = tree.cursor_to(&1);
    assert_eq!(cur.peek_prev(), Some((&0, &0)));
    assert_eq!(cur.peek_next(), Some((&2, &2)));
    assert_eq!(cur.key_value(), Some((&1, &1)));
}

#[test]
fn d_v_cursor_mut_to_mutates_value_in_place() {
    let mut tree = LinkedRBTree::new();
    for k in 0..3 {
        tree.try_insert(k, k).unwrap();
    }
    let mut cur = tree.cursor_mut_to(&1);
    *cur.value_mut().unwrap() = 99;
    assert_eq!(tree.get(&1), Some(&99));
}

#[test]
fn d_w_front_back_cursor_mut_match_ends() {
    let mut tree = LinkedRBTree::new();
    for k in 0..5 {
        tree.try_insert(k, k * 10).unwrap();
    }
    assert_eq!(tree.front_cursor_mut().key_value(), Some((&0, &0)));
    assert_eq!(tree.back_cursor_mut().key_value(), Some((&4, &40)));
}

// ---- drop semantics ---------------------------------------------------

struct DropCounter<'a>(&'a AtomicUsize);
impl<'a> Drop for DropCounter<'a> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, AtomicOrdering::SeqCst);
    }
}

#[test]
fn d_x_drop_deallocates_every_value() {
    let count = AtomicUsize::new(0);
    {
        let mut tree = LinkedRBTree::new();
        for k in 0..10 {
            count.fetch_add(1, AtomicOrdering::SeqCst);
            tree.try_insert(k, DropCounter(&count)).unwrap();
        }
        assert_eq!(count.load(AtomicOrdering::SeqCst), 10);
    }
    assert_eq!(count.load(AtomicOrdering::SeqCst), 0);
}

// ---- stress ------------------------------------------------------------

#[test]
fn d_y_interleaved_insert_remove_stays_sorted() {
    let mut tree = LinkedRBTree::new();
    let mut present: Vec<i32> = Vec::new();
    let mut seed = 0xDEADBEEFu32;

    for _ in 0..500 {
        let op = xorshift(&mut seed) % 3;
        let key = (xorshift(&mut seed) % 100) as i32;

        if op == 0 && !present.contains(&key) {
            tree.try_insert(key, key).unwrap();
            present.push(key);
        } else if !present.is_empty() {
            let idx = (xorshift(&mut seed) as usize) % present.len();
            let key = present.swap_remove(idx);
            assert_eq!(tree.remove(&key), Some((key, key)));
        }
    }

    present.sort_unstable();
    assert_eq!(ascending(&tree), present);
    assert_eq!(tree.len(), present.len());
}

#[test]
fn d_ad_overwrite_preserves_list_links() {
    let mut tree = LinkedRBTree::new();
    for k in 0..5 {
        tree.try_insert(k, k * 10).unwrap();
    }

    // overwrite a middle node and check its neighbors are unchanged
    let old = tree.try_insert(2, 999).unwrap();
    assert_eq!(old, Some(20));
    assert_eq!(tree.len(), 5);
    assert_eq!(tree.get(&2), Some(&999));

    let cur = tree.cursor_to(&2);
    assert_eq!(cur.peek_prev(), Some((&1, &10)));
    assert_eq!(cur.peek_next(), Some((&3, &30)));
    assert_eq!(ascending(&tree), vec![0, 1, 2, 3, 4]);
}

#[test]
fn d_ae_overwrite_head_keeps_head_pointer() {
    let mut tree = LinkedRBTree::new();
    for k in 0..5 {
        tree.try_insert(k, k).unwrap();
    }

    let old = tree.try_insert(0, 100).unwrap();
    assert_eq!(old, Some(0));
    assert_eq!(tree.front_cursor().key_value(), Some((&0, &100)));
    assert_eq!(ascending(&tree), vec![0, 1, 2, 3, 4]);
}

#[test]
fn d_af_overwrite_tail_keeps_tail_pointer() {
    let mut tree = LinkedRBTree::new();
    for k in 0..5 {
        tree.try_insert(k, k).unwrap();
    }

    let old = tree.try_insert(4, 400).unwrap();
    assert_eq!(old, Some(4));
    assert_eq!(tree.back_cursor().key_value(), Some((&4, &400)));
    assert_eq!(ascending(&tree), vec![0, 1, 2, 3, 4]);
}

#[test]
fn d_ag_overwrite_sole_node_keeps_head_and_tail() {
    let mut tree = LinkedRBTree::new();
    tree.try_insert(1, "a").unwrap();

    let old = tree.try_insert(1, "b").unwrap();
    assert_eq!(old, Some("a"));
    assert_eq!(tree.len(), 1);
    assert_eq!(tree.front_cursor().key_value(), Some((&1, &"b")));
    assert_eq!(tree.back_cursor().key_value(), Some((&1, &"b")));
}

#[test]
fn d_ah_repeated_overwrite_same_key_never_grows_list() {
    let mut tree = LinkedRBTree::new();
    tree.try_insert(1, 1).unwrap();
    tree.try_insert(2, 2).unwrap();

    for i in 0..50 {
        let old = tree.try_insert(1, i).unwrap();
        assert_eq!(old, Some(if i == 0 { 1 } else { i - 1 }));
    }

    assert_eq!(tree.len(), 2);
    assert_eq!(ascending(&tree), vec![1, 2]);
    assert_eq!(tree.get(&1), Some(&49));
}
