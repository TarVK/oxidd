"""Binary decision diagrams (BDDs, without complement edges)"""

__all__ = ["BDDManager", "BDDFunction"]

import collections.abc
from typing import Optional, Union

from _oxidd import ffi as _ffi
from _oxidd import lib as _lib
from typing_extensions import Never, Self, override

from . import protocols, util


class BDDManager(protocols.BooleanFunctionManager["BDDFunction"]):
    """Manager for binary decision diagrams (without complement edges)"""

    _mgr: ...  #: Wrapped FFI object (``oxidd_bdd_manager_t``)
    _vars = []

    def __init__(self, inner_node_capacity: int, apply_cache_size: int, threads: int):
        """Create a new manager

        :param inner_node_capacity: Maximum count of inner nodes
        :param apply_cache_capacity: Maximum count of apply cache entries
        :param threads: Worker thread count for the internal thread pool
        """
        self._mgr = _lib.oxidd_bdd_manager_new(
            inner_node_capacity, apply_cache_size, threads
        )

    @classmethod
    def _from_raw(cls, raw) -> Self:
        """Create a manager from a raw FFI object (``oxidd_bdd_manager_t``)"""
        manager = cls.__new__(cls)
        manager._mgr = raw
        return manager

    def __del__(self):
        _lib.oxidd_bdd_manager_unref(self._mgr)

    @override
    def __eq__(self, other: object) -> bool:
        """Check for referential equality"""
        return isinstance(other, BDDManager) and self._mgr._p == other._mgr._p

    @override
    def __hash__(self) -> int:
        return hash(self._mgr._p)

    @override
    def new_var(self, name: Optional[str]=None) -> "BDDFunction":
        if name == None:
            name = "x" + str(len(self._vars))
        var = BDDFunction._from_raw(_lib.oxidd_bdd_new_var(self._mgr))
        self._vars.append((name, var))
        return var

    @override
    def true(self) -> "BDDFunction":
        return BDDFunction._from_raw(_lib.oxidd_bdd_true(self._mgr))

    @override
    def false(self) -> "BDDFunction":
        return BDDFunction._from_raw(_lib.oxidd_bdd_false(self._mgr))

    @override
    def num_inner_nodes(self) -> int:
        return _lib.oxidd_bdd_num_inner_nodes(self._mgr)
    
    
    def export_dddmp(
        self,
        filename: str,
        func: Union[
            Union["BDDFunction", tuple[str, "BDDFunction"]], 
            list[Union["BDDFunction", tuple[str, "BDDFunction"]]]
        ], 
        dd_name: str = "viz",
        as_ascii: bool = True,
    ) -> None:
        """
            Export the decision diagram in to filename in DDDMP format.
            Multiple formats are allowed for `func` given some function `f` and `g`:
            - f
            - ("func_name", f)
            - [f, g]
            - [("func_name1", f), ("func_name2", g)]
        """
        tmp_functions, tmp_function_names, tmp_variables, tmp_variable_names = get_func_data(func, self._vars)

        _lib.oxidd_bdd_export_dddmp(
            filename.encode(),
            dd_name.encode(),
            tmp_functions,
            tmp_function_names,
            len(tmp_functions),
            tmp_variables,
            tmp_variable_names,
            len(tmp_variables),
            as_ascii,
        )

    def export_dot(
        self,
        filename: str, 
        func: Union[
            Union["BDDFunction", tuple[str, "BDDFunction"]], 
            list[Union["BDDFunction", tuple[str, "BDDFunction"]]]
        ], 
    ):
        """
            Export the decision diagram in to filename in Graphviz dot format.
            Multiple formats are allowed for `func` given some function `f` and `g`:
            - f
            - ("func_name", f)
            - [f, g]
            - [("func_name1", f), ("func_name2", g)]
        """
        tmp_functions, tmp_function_names, tmp_variables, tmp_variable_names = get_func_data(func, self._vars)

        _lib.oxidd_bdd_export_dot(
            filename.encode(),
            tmp_functions,
            tmp_function_names,
            len(tmp_functions),
            tmp_variables,
            tmp_variable_names,
            len(tmp_variables),
        )

    def visualize(
        self, 
        func: Union[
            Union["BDDFunction", tuple[str, "BDDFunction"]], 
            list[Union["BDDFunction", tuple[str, "BDDFunction"]]]
        ], 
        name: str = "viz", 
        host: str = 'http://127.0.0.1:8080'
    ) -> None:
        """
            Send the diagram to a host for visualization.
            Multiple formats are allowed for `func` given some function `f` and `g`:
            - f
            - ("func_name", f)
            - [f, g]
            - [("func_name1", f), ("func_name2", g)]
        """
        tmp_functions, tmp_function_names, tmp_variables, tmp_variable_names = get_func_data(func, self._vars)

        _lib.oxidd_bdd_visualize(
            name.encode(),
            host.encode(),
            tmp_functions,
            tmp_function_names,
            len(tmp_functions),
            tmp_variables,
            tmp_variable_names,
            len(tmp_variables),
        )

def get_func_data(
    func: Union[
        Union["BDDFunction", tuple[str, "BDDFunction"]], 
        list[Union["BDDFunction", tuple[str, "BDDFunction"]]]
    ],
    inp_vars: list[tuple[str, "BDDFunction"]]
):
    funcs = func if isinstance(func, list) else [func]
    named_funcs = [
        ((f'f{i}', f) if isinstance(f, BDDFunction) else f) 
        for (i, f) in enumerate(funcs)
    ]

    vars = [var for (name, var) in inp_vars]
    var_names = [name for (name, var) in inp_vars]
    
    
    tmp_variables = [var._func for var in vars]
    tmp_variable_names = [
        _ffi.new("char[]", name.encode()) for name in var_names
    ]
    tmp_functions = [
        f._func for (_, f) in named_funcs
    ]
    tmp_function_names = [
        _ffi.new("char[]", name.encode()) for (name, _) in named_funcs
    ]
    return (
        tmp_functions,
        tmp_function_names,
        tmp_variables,
        tmp_variable_names
    )



class BDDSubstitution:
    """Substitution mapping variables to replacement functions"""

    _subst: ...  #: Wrapped FFI object (``oxidd_bdd_substitution_t *``)

    def __init__(
        self, pairs: collections.abc.Iterable[tuple["BDDFunction", "BDDFunction"]]
    ):
        """Create a new substitution object for BDDs

        See :meth:`abc.FunctionSubst.make_substitution` fore more details.
        """
        self._subst = _lib.oxidd_bdd_substitution_new(
            len(pairs) if isinstance(pairs, collections.abc.Sized) else 0
        )
        for v, r in pairs:
            _lib.oxidd_bdd_substitution_add_pair(self._subst, v._func, r._func)

    def __del__(self):
        _lib.oxidd_bdd_substitution_free(self._subst)


class BDDFunction(
    protocols.BooleanFunctionQuant, protocols.FunctionSubst[BDDSubstitution]
):
    """Boolean function represented as a simple binary decision diagram (BDD)

    All operations constructing BDDs may throw a :exc:`MemoryError` in case they
    run out of memory.
    """

    _func: ...  #: Wrapped FFI object (``oxidd_bdd_t``)

    def __init__(self, _: Never):
        """Private constructor

        Functions cannot be instantiated directly and must be created from the
        :class:`BDDManager` or by combining existing functions instead.
        """
        raise RuntimeError(
            "Functions cannot be instantiated directly and must be created "
            "from the BDDManager and by combining existing functions instead."
        )

    @classmethod
    def _from_raw(cls, raw) -> Self:
        """Create a BDD function from a raw FFI object (``oxidd_bdd_t``)"""
        if raw._p == _ffi.NULL:
            raise MemoryError("OxiDD BDD operation ran out of memory")
        function = cls.__new__(cls)
        function._func = raw
        return function

    def __del__(self):
        _lib.oxidd_bdd_unref(self._func)

    @override
    def __eq__(self, other: object) -> bool:
        """Check if ``other`` references the same node in the same manager

        Since BDDs are a strong canonical form, ``self`` and ``other`` are
        semantically equal (i.e., represent the same Boolean functions) if and
        only if this method returns ``True``.
        """
        return (
            isinstance(other, BDDFunction)
            and self._func._p == other._func._p
            and self._func._i == other._func._i
        )

    def __lt__(self, other: Self) -> bool:
        """Check if ``self`` is less than ``other`` according to an arbitrary
        total order

        The following restrictions apply: Assuming two functions ``f``, ``g``
        with ``f < g``, then if either ``f`` or ``g`` is deleted and recreated
        later on, ``f < g`` does not necessarily hold anymore. Moreover, assume
        `f < g` and two structurally equivalent functions ``f2``, ``g2`` in
        different managers (i.e., ``f ≅ f2`` but ``f != f2``, and ``g ≅ g2`` but
        ``g != g2``), then ``f2 < g2`` does not necessarily hold.
        """
        return (self._func._p, self._func._i) < (other._func._p, other._func._i)

    def __gt__(self, other: Self) -> bool:
        """Same as ``other < self``"""
        return (self._func._p, self._func._i) > (other._func._p, other._func._i)

    def __le__(self, other: Self) -> bool:
        """Same as ``not self > other``"""
        return (self._func._p, self._func._i) <= (other._func._p, other._func._i)

    def __ge__(self, other: Self) -> bool:
        """Same as ``not self < other``"""
        return (self._func._p, self._func._i) >= (other._func._p, other._func._i)
    
    def export_dddmp(
        self,
        dd_name: str,
        filename: str,
        function_name: str,
        variables: list[Self],
        variable_names: list[str],
        as_ascii: bool,
    ) -> None:
        """Export the decision diagram in to filename in DDDMP format"""
        tmp_variables = [var._func for var in variables]
        tmp_variable_names = [
            _ffi.new("char[]", name.encode()) for name in variable_names
        ]
        tmp_functions = [
            self._func
        ]
        tmp_function_names = [
            _ffi.new("char[]", function_name.encode())
        ]

        _lib.oxidd_bdd_export_dddmp(
            filename.encode(),
            dd_name.encode(),
            tmp_functions,
            tmp_function_names,
            len(tmp_functions),
            tmp_variables,
            tmp_variable_names,
            len(variables),
            as_ascii,
        )

    def export_dot(
        self,
        filename: str,
        function_name: str,
        variables: list[Self],
        variable_names: list[str],
    ):
        """Export the decision diagram in to filename in Graphviz dot format"""
        tmp_variables = [var._func for var in variables]
        tmp_variable_names = [
            _ffi.new("char[]", name.encode()) for name in variable_names
        ]
        tmp_functions = [
            self._func
        ]
        tmp_function_names = [
            _ffi.new("char[]", function_name.encode())
        ]

        _lib.oxidd_bdd_export_dot(
            filename.encode(),
            tmp_functions,
            tmp_function_names,
            len(tmp_functions),
            tmp_variables,
            tmp_variable_names,
            len(variables),
        )
        
    def visualize(
        self,
        function_name: str,
        variables: list[Self],
        variable_names: list[str],
        name: str = "viz", 
        host: str = 'http://127.0.0.1:8080'
    ):
        """Send the diagram to a host for visualization"""
        tmp_variables = [var._func for var in variables]
        tmp_variable_names = [
            _ffi.new("char[]", name.encode()) for name in variable_names
        ]
        tmp_functions = [
            self._func
        ]
        tmp_function_names = [
            _ffi.new("char[]", function_name.encode())
        ]

        _lib.oxidd_bdd_visualize(
            name.encode(),
            host.encode(),
            tmp_functions,
            tmp_function_names,
            len(tmp_functions),
            tmp_variables,
            tmp_variable_names,
            len(variables),
        )

    @override
    def __hash__(self) -> int:
        return hash((self._func._p, self._func._i))

    @property
    @override
    def manager(self) -> BDDManager:
        return BDDManager._from_raw(_lib.oxidd_bdd_containing_manager(self._func))

    @override
    def cofactors(self) -> tuple[Self, Self]:
        raw_pair = _lib.oxidd_bdd_cofactors(self._func)
        return (
            self.__class__._from_raw(raw_pair.first),
            self.__class__._from_raw(raw_pair.second),
        )

    @override
    def cofactor_true(self) -> Self:
        return self.__class__._from_raw(_lib.oxidd_bdd_cofactor_true(self._func))

    @override
    def cofactor_false(self) -> Self:
        return self.__class__._from_raw(_lib.oxidd_bdd_cofactor_false(self._func))

    @override
    def __invert__(self) -> Self:
        return self.__class__._from_raw(_lib.oxidd_bdd_not(self._func))

    @override
    def __and__(self, rhs: Self) -> Self:
        assert (
            self._func._p == rhs._func._p
        ), "`self` and `rhs` must belong to the same manager"
        return self.__class__._from_raw(_lib.oxidd_bdd_and(self._func, rhs._func))

    @override
    def __or__(self, rhs: Self) -> Self:
        assert (
            self._func._p == rhs._func._p
        ), "`self` and `rhs` must belong to the same manager"
        return self.__class__._from_raw(_lib.oxidd_bdd_or(self._func, rhs._func))

    @override
    def __xor__(self, rhs: Self) -> Self:
        assert (
            self._func._p == rhs._func._p
        ), "`self` and `rhs` must belong to the same manager"
        return self.__class__._from_raw(_lib.oxidd_bdd_xor(self._func, rhs._func))

    @override
    def nand(self, rhs: Self) -> Self:
        assert (
            self._func._p == rhs._func._p
        ), "`self` and `rhs` must belong to the same manager"
        return self.__class__._from_raw(_lib.oxidd_bdd_nand(self._func, rhs._func))

    @override
    def nor(self, rhs: Self) -> Self:
        assert (
            self._func._p == rhs._func._p
        ), "`self` and `rhs` must belong to the same manager"
        return self.__class__._from_raw(_lib.oxidd_bdd_nor(self._func, rhs._func))

    @override
    def equiv(self, rhs: Self) -> Self:
        assert (
            self._func._p == rhs._func._p
        ), "`self` and `rhs` must belong to the same manager"
        return self.__class__._from_raw(_lib.oxidd_bdd_equiv(self._func, rhs._func))

    @override
    def imp(self, rhs: Self) -> Self:
        assert (
            self._func._p == rhs._func._p
        ), "`self` and `rhs` must belong to the same manager"
        return self.__class__._from_raw(_lib.oxidd_bdd_imp(self._func, rhs._func))

    @override
    def imp_strict(self, rhs: Self) -> Self:
        assert (
            self._func._p == rhs._func._p
        ), "`self` and `rhs` must belong to the same manager"
        return self.__class__._from_raw(
            _lib.oxidd_bdd_imp_strict(self._func, rhs._func)
        )

    @override
    def ite(self, t: Self, e: Self) -> Self:
        assert (
            self._func._p == t._func._p == e._func._p
        ), "`self` and `t` and `e` must belong to the same manager"
        return self.__class__._from_raw(
            _lib.oxidd_bdd_ite(self._func, t._func, e._func)
        )

    @override
    @classmethod
    def make_substitution(
        cls, pairs: collections.abc.Iterable[tuple[Self, Self]]
    ) -> BDDSubstitution:
        return BDDSubstitution(pairs)

    @override
    def substitute(self, substitution: BDDSubstitution) -> Self:
        return self.__class__._from_raw(
            _lib.oxidd_bdd_substitute(self._func, substitution._subst)
        )

    @override
    def forall(self, vars: Self) -> Self:
        assert (
            self._func._p == vars._func._p
        ), "`self` and `vars` must belong to the same manager"
        return self.__class__._from_raw(_lib.oxidd_bdd_forall(self._func, vars._func))

    @override
    def exist(self, vars: Self) -> Self:
        assert (
            self._func._p == vars._func._p
        ), "`self` and `vars` must belong to the same manager"
        return self.__class__._from_raw(_lib.oxidd_bdd_exist(self._func, vars._func))

    @override
    def unique(self, vars: Self) -> Self:
        assert (
            self._func._p == vars._func._p
        ), "`self` and `vars` must belong to the same manager"
        return self.__class__._from_raw(_lib.oxidd_bdd_unique(self._func, vars._func))

    @override
    def apply_forall(self, op: util.BooleanOperator, rhs: Self, vars: Self):
        if not isinstance(op, util.BooleanOperator):
            # If op were an arbitrary integer that is not part of the enum,
            # this would lead to undefined behavior
            raise ValueError("`op` must be a BooleanOperator")
        assert (
            self._func._p == rhs._func._p == vars._func._p
        ), "`self`, `rhs`, and `vars` must belong to the same manager"
        return self.__class__._from_raw(
            _lib.oxidd_bdd_apply_forall(op, self._func, rhs._func, vars._func)
        )

    @override
    def apply_exist(self, op: util.BooleanOperator, rhs: Self, vars: Self):
        if not isinstance(op, util.BooleanOperator):
            raise ValueError("`op` must be a BooleanOperator")
        assert (
            self._func._p == rhs._func._p == vars._func._p
        ), "`self`, `rhs`, and `vars` must belong to the same manager"
        return self.__class__._from_raw(
            _lib.oxidd_bdd_apply_exist(op, self._func, rhs._func, vars._func)
        )

    @override
    def apply_unique(self, op: util.BooleanOperator, rhs: Self, vars: Self):
        if not isinstance(op, util.BooleanOperator):
            raise ValueError("`op` must be a BooleanOperator")
        assert (
            self._func._p == rhs._func._p == vars._func._p
        ), "`self`, `rhs`, and `vars` must belong to the same manager"
        return self.__class__._from_raw(
            _lib.oxidd_bdd_apply_unique(op, self._func, rhs._func, vars._func)
        )

    @override
    def node_count(self) -> int:
        return int(_lib.oxidd_bdd_node_count(self._func))

    @override
    def satisfiable(self) -> bool:
        return bool(_lib.oxidd_bdd_satisfiable(self._func))

    @override
    def valid(self) -> bool:
        return bool(_lib.oxidd_bdd_valid(self._func))

    @override
    def sat_count_float(self, vars: int) -> float:
        return float(_lib.oxidd_bdd_sat_count_double(self._func, vars))

    @override
    def pick_cube(self) -> Optional[util.Assignment]:
        raw = _lib.oxidd_bdd_pick_cube(self._func)
        return util.Assignment._from_raw(raw) if raw.len > 0 else None

    @override
    def pick_cube_symbolic(self) -> Self:
        return self.__class__._from_raw(_lib.oxidd_bdd_pick_cube_symbolic(self._func))

    @override
    def pick_cube_symbolic_set(self, literal_set: Self) -> Self:
        assert (
            self._func._p == literal_set._func._p
        ), "`self` and `literal_set` must belong to the same manager"
        return self.__class__._from_raw(
            _lib.oxidd_bdd_pick_cube_symbolic_set(self._func, literal_set._func)
        )

    @override
    def eval(self, args: collections.abc.Collection[tuple[Self, bool]]) -> bool:
        n = len(args)
        c_args = util._alloc("oxidd_bdd_bool_pair_t[]", n)
        for c_arg, (f, v) in zip(c_args, args):
            c_arg.func = f._func
            c_arg.val = v

        return bool(_lib.oxidd_bdd_eval(self._func, c_args, n))

