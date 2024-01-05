//! Recursive single-threaded apply algorithms

use std::collections::HashMap;
use std::hash::BuildHasher;

use bitvec::vec::BitVec;

use oxidd_core::function::BooleanFunction;
use oxidd_core::function::BooleanFunctionQuant;
use oxidd_core::function::Function;
use oxidd_core::util::Borrowed;
use oxidd_core::util::EdgeDropGuard;
use oxidd_core::util::OptBool;
use oxidd_core::util::SatCountNumber;
use oxidd_core::ApplyCache;
use oxidd_core::Edge;
use oxidd_core::HasApplyCache;
use oxidd_core::HasLevel;
use oxidd_core::InnerNode;
use oxidd_core::LevelNo;
use oxidd_core::Manager;
use oxidd_core::Node;
use oxidd_core::NodeID;
use oxidd_core::Tag;
use oxidd_derive::Function;
use oxidd_dump::dot::DotStyle;

use super::*;

/// Recursively apply the 'not' operator to `f`
pub(super) fn apply_not<M>(manager: &M, f: Borrowed<M::Edge>) -> AllocResult<M::Edge>
where
    M: Manager<Terminal = BDDTerminal> + HasApplyCache<M, Operator = BDDOp>,
    M::InnerNode: HasLevel,
{
    stat!(call BDDOp::Not);
    let node = match manager.get_node(&*f) {
        Node::Inner(node) => node,
        Node::Terminal(t) => return Ok(manager.get_terminal(!*t.borrow()).unwrap()),
    };

    // Query apply cache
    stat!(cache_query BDDOp::Not);
    if let Some(h) = manager
        .apply_cache()
        .get(manager, BDDOp::Not, &[f.borrowed()])
    {
        stat!(cache_hit BDDOp::Not);
        return Ok(h);
    }

    let (f0, f1) = collect_children(node);
    let level = node.level();

    let t = EdgeDropGuard::new(manager, apply_not(manager, f0)?);
    let e = EdgeDropGuard::new(manager, apply_not(manager, f1)?);
    let h = reduce(manager, level, t.into_edge(), e.into_edge(), BDDOp::Not)?;

    // Add to apply cache
    manager
        .apply_cache()
        .add(manager, BDDOp::Not, &[f.borrowed()], h.borrowed());

    Ok(h)
}

/// Recursively apply the binary operator `OP` to `f` and `g`
///
/// We use a `const` parameter `OP` to have specialized version of this function
/// for each operator.
pub(super) fn apply_bin<M, const OP: u8>(
    manager: &M,
    f: Borrowed<M::Edge>,
    g: Borrowed<M::Edge>,
) -> AllocResult<M::Edge>
where
    M: Manager<Terminal = BDDTerminal> + HasApplyCache<M, Operator = BDDOp>,
    M::InnerNode: HasLevel,
{
    stat!(call OP);
    let (operator, op1, op2) = match terminal_bin::<M, OP>(manager, &*f, &*g) {
        Operation::Binary(o, op1, op2) => (o, op1, op2),
        Operation::Not(f) => {
            return apply_not(manager, f);
        }
        Operation::Done(h) => return Ok(h),
    };

    // Query apply cache
    stat!(cache_query OP);
    if let Some(h) = manager
        .apply_cache()
        .get(manager, operator, &[op1.borrowed(), op2.borrowed()])
    {
        stat!(cache_hit OP);
        return Ok(h);
    }

    let fnode = manager.get_node(&*f).unwrap_inner();
    let gnode = manager.get_node(&*g).unwrap_inner();
    let flevel = fnode.level();
    let glevel = gnode.level();
    let level = std::cmp::min(flevel, glevel);

    // Collect cofactors of all top-most nodes
    let (f0, f1) = if flevel == level {
        collect_children(fnode)
    } else {
        (f.borrowed(), f.borrowed())
    };
    let (g0, g1) = if glevel == level {
        collect_children(gnode)
    } else {
        (g.borrowed(), g.borrowed())
    };

    let t = EdgeDropGuard::new(manager, apply_bin::<M, OP>(manager, f0, g0)?);
    let e = EdgeDropGuard::new(manager, apply_bin::<M, OP>(manager, f1, g1)?);
    let h = reduce(manager, level, t.into_edge(), e.into_edge(), operator)?;

    // Add to apply cache
    manager
        .apply_cache()
        .add(manager, operator, &[op1, op2], h.borrowed());

    Ok(h)
}

/// Recursively apply the if-then-else operator (`if f { g } else { h }`)
pub(super) fn apply_ite_rec<M>(
    manager: &M,
    f: Borrowed<M::Edge>,
    g: Borrowed<M::Edge>,
    h: Borrowed<M::Edge>,
) -> AllocResult<M::Edge>
where
    M: Manager<Terminal = BDDTerminal> + HasApplyCache<M, Operator = BDDOp>,
    M::InnerNode: HasLevel,
{
    use BDDTerminal::*;
    stat!(call BDDOp::Ite);

    // Terminal cases
    if &*g == &*h {
        return Ok(manager.clone_edge(&*g));
    }
    if &*f == &*g {
        return apply_bin::<M, { BDDOp::Or as u8 }>(manager, f, h);
    }
    if &*f == &*h {
        return apply_bin::<M, { BDDOp::And as u8 }>(manager, f, g);
    }
    let fnode = match manager.get_node(&*f) {
        Node::Inner(n) => n,
        Node::Terminal(t) => {
            return Ok(manager.clone_edge(&*if *t.borrow() == True { g } else { h }))
        }
    };
    let (gnode, hnode) = match (manager.get_node(&*g), manager.get_node(&*h)) {
        (Node::Inner(gn), Node::Inner(hn)) => (gn, hn),
        (Node::Terminal(t), Node::Inner(_)) => {
            return match t.borrow() {
                True => apply_bin::<M, { BDDOp::Or as u8 }>(manager, f, h),
                False => apply_bin::<M, { BDDOp::ImpStrict as u8 }>(manager, f, h),
            };
        }
        (Node::Inner(_), Node::Terminal(t)) => {
            return match t.borrow() {
                True => apply_bin::<M, { BDDOp::Imp as u8 }>(manager, f, g),
                False => apply_bin::<M, { BDDOp::And as u8 }>(manager, f, g),
            };
        }
        (Node::Terminal(gt), Node::Terminal(_ht)) => {
            debug_assert_ne!(gt.borrow(), _ht.borrow()); // g == h is handled above
            return match gt.borrow() {
                False => apply_not(manager, f),      // if f { ⊥ } else { ⊤ }
                True => Ok(manager.clone_edge(&*f)), // if f { ⊤ } else { ⊥ }
            };
        }
    };

    // Query apply cache
    stat!(cache_query BDDOp::Ite);
    if let Some(res) = manager.apply_cache().get(
        manager,
        BDDOp::Ite,
        &[f.borrowed(), g.borrowed(), h.borrowed()],
    ) {
        stat!(cache_hit BDDOp::Ite);
        return Ok(res);
    }

    // Get the top-most level of the three
    let flevel = fnode.level();
    let glevel = gnode.level();
    let hlevel = hnode.level();
    let level = std::cmp::min(std::cmp::min(flevel, glevel), hlevel);

    // Collect cofactors of all top-most nodes
    let (f0, f1) = if flevel == level {
        collect_children(fnode)
    } else {
        (f.borrowed(), f.borrowed())
    };
    let (g0, g1) = if glevel == level {
        collect_children(gnode)
    } else {
        (g.borrowed(), g.borrowed())
    };
    let (h0, h1) = if hlevel == level {
        collect_children(hnode)
    } else {
        (h.borrowed(), h.borrowed())
    };

    let t = EdgeDropGuard::new(manager, apply_ite_rec(manager, f0, g0, h0)?);
    let e = EdgeDropGuard::new(manager, apply_ite_rec(manager, f1, g1, h1)?);
    let res = reduce(manager, level, t.into_edge(), e.into_edge(), BDDOp::Ite)?;

    manager
        .apply_cache()
        .add(manager, BDDOp::Ite, &[f, g, h], res.borrowed());

    Ok(res)
}

/// Compute the quantification `Q` over `vars`
///
/// Note that `Q` is one of `BDDOp::And`, `BDDOp::Or`, and `BDDOp::Xor` as `u8`.
/// This saves us another case distinction in the code (would not be present at
/// runtime).
pub(super) fn quant<M, const Q: u8>(
    manager: &M,
    f: Borrowed<M::Edge>,
    vars: Borrowed<M::Edge>,
) -> AllocResult<M::Edge>
where
    M: Manager<Terminal = BDDTerminal> + HasApplyCache<M, Operator = BDDOp>,
    M::InnerNode: HasLevel,
{
    let operator = match () {
        _ if Q == BDDOp::And as u8 => BDDOp::Forall,
        _ if Q == BDDOp::Or as u8 => BDDOp::Exist,
        _ if Q == BDDOp::Xor as u8 => BDDOp::Unique,
        _ => unreachable!("invalid quantifier"),
    };

    stat!(call operator);
    // Terminal cases
    let fnode = match manager.get_node(&*f) {
        Node::Inner(n) => n,
        Node::Terminal(_) => return Ok(manager.clone_edge(&*f)),
    };
    let flevel = fnode.level();

    // We can ignore all variables above the top-most variable. Removing them
    // before querying the apply cache should increase the hit ratio by a lot.
    let vars = crate::set_pop(manager, vars, flevel);
    let vlevel = match manager.get_node(&*vars) {
        Node::Inner(n) => n.level(),
        Node::Terminal(_) => return Ok(manager.clone_edge(&*f)),
    };
    debug_assert!(flevel <= vlevel);
    let vars = vars.borrowed();

    // Query apply cache
    stat!(cache_query operator);
    if let Some(res) =
        manager
            .apply_cache()
            .get(manager, operator, &[f.borrowed(), vars.borrowed()])
    {
        stat!(cache_hit operator);
        return Ok(res);
    }

    let (f0, f1) = collect_children(fnode);
    let t = EdgeDropGuard::new(manager, quant::<M, Q>(manager, f0, vars.borrowed())?);
    let e = EdgeDropGuard::new(manager, quant::<M, Q>(manager, f1, vars.borrowed())?);

    let res = if flevel == vlevel {
        apply_bin::<M, Q>(manager, t.borrowed(), e.borrowed())
    } else {
        reduce(manager, flevel, t.into_edge(), e.into_edge(), operator)
    }?;

    manager
        .apply_cache()
        .add(manager, operator, &[f, vars], res.borrowed());

    Ok(res)
}

// --- Function Interface ------------------------------------------------------

/// Boolean function backed by a binary decision diagram
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Function, Debug)]
#[repr(transparent)]
pub struct BDDFunction<F: Function>(F);

impl<F: Function> From<F> for BDDFunction<F> {
    #[inline(always)]
    fn from(value: F) -> Self {
        BDDFunction(value)
    }
}

impl<F: Function> BDDFunction<F> {
    /// Convert `self` into the underlying [`Function`]
    #[inline(always)]
    pub fn into_inner(self) -> F {
        self.0
    }
}

impl<F: Function> BooleanFunction for BDDFunction<F>
where
    for<'id> F::Manager<'id>: Manager<Terminal = BDDTerminal> + HasBDDOpApplyCache<F::Manager<'id>>,
    for<'id> <F::Manager<'id> as Manager>::InnerNode: HasLevel,
{
    #[inline]
    fn new_var<'id>(manager: &mut Self::Manager<'id>) -> AllocResult<Self> {
        let f0 = manager.get_terminal(BDDTerminal::True).unwrap();
        let f1 = manager.get_terminal(BDDTerminal::False).unwrap();
        let edge = manager.add_level(|level| InnerNode::new(level, [f0, f1]))?;
        Ok(Self::from_edge(manager, edge))
    }

    #[inline]
    fn f_edge<'id>(manager: &Self::Manager<'id>) -> <Self::Manager<'id> as Manager>::Edge {
        manager.get_terminal(BDDTerminal::False).unwrap()
    }
    #[inline]
    fn t_edge<'id>(manager: &Self::Manager<'id>) -> <Self::Manager<'id> as Manager>::Edge {
        manager.get_terminal(BDDTerminal::True).unwrap()
    }

    #[inline]
    fn not_edge<'id>(
        manager: &Self::Manager<'id>,
        edge: &<Self::Manager<'id> as Manager>::Edge,
    ) -> AllocResult<<Self::Manager<'id> as Manager>::Edge> {
        apply_not(manager, edge.borrowed())
    }

    #[inline]
    fn and_edge<'id>(
        manager: &Self::Manager<'id>,
        lhs: &<Self::Manager<'id> as Manager>::Edge,
        rhs: &<Self::Manager<'id> as Manager>::Edge,
    ) -> AllocResult<<Self::Manager<'id> as Manager>::Edge> {
        apply_bin::<_, { BDDOp::And as u8 }>(manager, lhs.borrowed(), rhs.borrowed())
    }
    #[inline]
    fn or_edge<'id>(
        manager: &Self::Manager<'id>,
        lhs: &<Self::Manager<'id> as Manager>::Edge,
        rhs: &<Self::Manager<'id> as Manager>::Edge,
    ) -> AllocResult<<Self::Manager<'id> as Manager>::Edge> {
        apply_bin::<_, { BDDOp::Or as u8 }>(manager, lhs.borrowed(), rhs.borrowed())
    }
    #[inline]
    fn nand_edge<'id>(
        manager: &Self::Manager<'id>,
        lhs: &<Self::Manager<'id> as Manager>::Edge,
        rhs: &<Self::Manager<'id> as Manager>::Edge,
    ) -> AllocResult<<Self::Manager<'id> as Manager>::Edge> {
        apply_bin::<_, { BDDOp::Nand as u8 }>(manager, lhs.borrowed(), rhs.borrowed())
    }
    #[inline]
    fn nor_edge<'id>(
        manager: &Self::Manager<'id>,
        lhs: &<Self::Manager<'id> as Manager>::Edge,
        rhs: &<Self::Manager<'id> as Manager>::Edge,
    ) -> AllocResult<<Self::Manager<'id> as Manager>::Edge> {
        apply_bin::<_, { BDDOp::Nor as u8 }>(manager, lhs.borrowed(), rhs.borrowed())
    }
    #[inline]
    fn xor_edge<'id>(
        manager: &Self::Manager<'id>,
        lhs: &<Self::Manager<'id> as Manager>::Edge,
        rhs: &<Self::Manager<'id> as Manager>::Edge,
    ) -> AllocResult<<Self::Manager<'id> as Manager>::Edge> {
        apply_bin::<_, { BDDOp::Xor as u8 }>(manager, lhs.borrowed(), rhs.borrowed())
    }
    #[inline]
    fn equiv_edge<'id>(
        manager: &Self::Manager<'id>,
        lhs: &<Self::Manager<'id> as Manager>::Edge,
        rhs: &<Self::Manager<'id> as Manager>::Edge,
    ) -> AllocResult<<Self::Manager<'id> as Manager>::Edge> {
        apply_bin::<_, { BDDOp::Equiv as u8 }>(manager, lhs.borrowed(), rhs.borrowed())
    }
    #[inline]
    fn imp_edge<'id>(
        manager: &Self::Manager<'id>,
        lhs: &<Self::Manager<'id> as Manager>::Edge,
        rhs: &<Self::Manager<'id> as Manager>::Edge,
    ) -> AllocResult<<Self::Manager<'id> as Manager>::Edge> {
        apply_bin::<_, { BDDOp::Imp as u8 }>(manager, lhs.borrowed(), rhs.borrowed())
    }
    #[inline]
    fn imp_strict_edge<'id>(
        manager: &Self::Manager<'id>,
        lhs: &<Self::Manager<'id> as Manager>::Edge,
        rhs: &<Self::Manager<'id> as Manager>::Edge,
    ) -> AllocResult<<Self::Manager<'id> as Manager>::Edge> {
        apply_bin::<_, { BDDOp::ImpStrict as u8 }>(manager, lhs.borrowed(), rhs.borrowed())
    }

    #[inline]
    fn ite_edge<'id>(
        manager: &Self::Manager<'id>,
        if_edge: &<Self::Manager<'id> as Manager>::Edge,
        then_edge: &<Self::Manager<'id> as Manager>::Edge,
        else_edge: &<Self::Manager<'id> as Manager>::Edge,
    ) -> AllocResult<<Self::Manager<'id> as Manager>::Edge> {
        apply_ite_rec(
            manager,
            if_edge.borrowed(),
            then_edge.borrowed(),
            else_edge.borrowed(),
        )
    }

    fn sat_count_edge<'id, N: SatCountNumber, S: BuildHasher>(
        manager: &Self::Manager<'id>,
        edge: &<Self::Manager<'id> as Manager>::Edge,
        vars: LevelNo,
        cache: &mut HashMap<NodeID, N, S>,
    ) -> N {
        fn inner<M: Manager<Terminal = BDDTerminal>, N: SatCountNumber, S: BuildHasher>(
            manager: &M,
            e: Borrowed<M::Edge>,
            terminal_val: &N,
            cache: &mut HashMap<NodeID, N, S>,
        ) -> N {
            let node = match manager.get_node(&*e) {
                Node::Inner(node) => node,
                Node::Terminal(t) => {
                    return if *t.borrow() == BDDTerminal::True {
                        terminal_val.clone()
                    } else {
                        N::from(0u32)
                    };
                }
            };
            let node_id = e.node_id();
            if let Some(n) = cache.get(&node_id) {
                return n.clone();
            }
            let (e0, e1) = collect_children(node);
            let mut n = inner(manager, e0, terminal_val, cache);
            n += &inner(manager, e1, terminal_val, cache);
            n >>= 1u32;
            cache.insert(node_id, n.clone());
            n
        }

        let mut terminal_val = N::from(1u32);
        terminal_val <<= vars;
        inner(manager, edge.borrowed(), &terminal_val, cache)
    }

    fn pick_cube_edge<'id, 'a, I>(
        manager: &'a Self::Manager<'id>,
        edge: &'a <Self::Manager<'id> as Manager>::Edge,
        order: impl IntoIterator<IntoIter = I>,
        choice: impl FnMut(&Self::Manager<'id>, &<Self::Manager<'id> as Manager>::Edge) -> bool,
    ) -> Option<Vec<OptBool>>
    where
        I: ExactSizeIterator<Item = &'a <Self::Manager<'id> as Manager>::Edge>,
    {
        #[inline] // this function is tail-recursive
        fn inner<M: Manager<Terminal = BDDTerminal>>(
            manager: &M,
            edge: Borrowed<M::Edge>,
            cube: &mut [OptBool],
            mut choice: impl FnMut(&M, &M::Edge) -> bool,
        ) where
            M::InnerNode: HasLevel,
        {
            let Node::Inner(node) = manager.get_node(&*edge) else {
                return;
            };
            let (t, e) = collect_children(node);
            let c = if manager.get_node(&*t).is_terminal(&BDDTerminal::False) {
                false
            } else if manager.get_node(&*e).is_terminal(&BDDTerminal::False) {
                true
            } else {
                choice(manager, &*edge)
            };
            cube[node.level() as usize] = OptBool::from(c);
            inner(manager, if c { t } else { e }, cube, choice);
        }

        let order = order.into_iter();
        debug_assert!(
            order.len() == 0 || order.len() == manager.num_levels() as usize,
            "order must be empty or contain all variables"
        );

        match manager.get_node(edge) {
            Node::Inner(_) => {}
            Node::Terminal(t) => {
                return match *t.borrow() {
                    BDDTerminal::False => None,
                    BDDTerminal::True => Some(vec![OptBool::None; manager.num_levels() as usize]),
                }
            }
        }

        let mut cube = vec![OptBool::None; manager.num_levels() as usize];
        inner(manager, edge.borrowed(), &mut cube, choice);

        Some(if order.len() != 0 {
            cube
        } else {
            order
                .map(|e| cube[manager.get_node(e).unwrap_inner().level() as usize])
                .collect()
        })
    }

    fn eval_edge<'id, 'a>(
        manager: &'a Self::Manager<'id>,
        edge: &'a <Self::Manager<'id> as Manager>::Edge,
        env: impl IntoIterator<Item = (&'a <Self::Manager<'id> as Manager>::Edge, bool)>,
    ) -> bool {
        let mut vals = BitVec::new();
        vals.resize(manager.num_levels() as usize, false);
        for (edge, val) in env {
            let node = manager
                .get_node(edge)
                .expect_inner("edges in `env` must refer to inner nodes");
            vals.set(node.level() as usize, val);
        }

        #[inline] // this function is tail-recursive
        fn inner<M>(manager: &M, edge: Borrowed<M::Edge>, vals: BitVec) -> bool
        where
            M: Manager<Terminal = BDDTerminal>,
            M::InnerNode: HasLevel,
        {
            match manager.get_node(&*edge) {
                Node::Inner(node) => {
                    let edge = node.child((!vals[node.level() as usize]) as usize);
                    inner(manager, edge, vals)
                }
                Node::Terminal(t) => *t.borrow() == BDDTerminal::True,
            }
        }

        inner(manager, edge.borrowed(), vals)
    }
}

impl<F: Function> BooleanFunctionQuant for BDDFunction<F>
where
    for<'id> F::Manager<'id>: Manager<Terminal = BDDTerminal> + HasBDDOpApplyCache<F::Manager<'id>>,
    for<'id> <F::Manager<'id> as Manager>::InnerNode: HasLevel,
{
    #[inline]
    fn forall_edge<'id>(
        manager: &Self::Manager<'id>,
        root: &<Self::Manager<'id> as Manager>::Edge,
        vars: &<Self::Manager<'id> as Manager>::Edge,
    ) -> AllocResult<<Self::Manager<'id> as Manager>::Edge> {
        quant::<_, { BDDOp::And as u8 }>(manager, root.borrowed(), vars.borrowed())
    }

    #[inline]
    fn exist_edge<'id>(
        manager: &Self::Manager<'id>,
        root: &<Self::Manager<'id> as Manager>::Edge,
        vars: &<Self::Manager<'id> as Manager>::Edge,
    ) -> AllocResult<<Self::Manager<'id> as Manager>::Edge> {
        quant::<_, { BDDOp::Or as u8 }>(manager, root.borrowed(), vars.borrowed())
    }

    #[inline]
    fn unique_edge<'id>(
        manager: &Self::Manager<'id>,
        root: &<Self::Manager<'id> as Manager>::Edge,
        vars: &<Self::Manager<'id> as Manager>::Edge,
    ) -> AllocResult<<Self::Manager<'id> as Manager>::Edge> {
        quant::<_, { BDDOp::Xor as u8 }>(manager, root.borrowed(), vars.borrowed())
    }
}

impl<F: Function, T: Tag> DotStyle<T> for BDDFunction<F> {}
