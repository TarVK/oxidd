
import oxidd
from oxidd.protocols import (
    BooleanFunction,
    BooleanFunctionManager,
    BooleanFunctionQuant,
    FunctionSubst,
)
from oxidd.util import BooleanOperator

def test_visualize():
    mgr = oxidd.bdd.BDDManager(1024, 1024, 1)
    x = mgr.new_var("x")
    y = mgr.new_var("y")
    z = mgr.new_var("z")
    w = mgr.new_var("w")
    f = x & y & ~z & w
    mgr.visualize([f, z & w])