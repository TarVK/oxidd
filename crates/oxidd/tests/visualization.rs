use oxidd::bdd::BDDFunction;
use oxidd::{BooleanFunction, Function, ManagerRef};
use oxidd_dump::visualize::visualize;

#[test]
fn bdd_visualization() {
    let mref = oxidd::bdd::new_manager(1024, 128, 2);

    let (x0, x1, x2, x3, x4) = mref.with_manager_exclusive(|manager| {
        (
            BDDFunction::new_var(manager).unwrap(),
            BDDFunction::new_var(manager).unwrap(),
            BDDFunction::new_var(manager).unwrap(),
            BDDFunction::new_var(manager).unwrap(),
            BDDFunction::new_var(manager).unwrap(),
        )
    });

    let g = x1.and(&x0).unwrap().not().unwrap();
    let p = x2.xor(&x3).unwrap();
    let f = x4
        .and(&g)
        .unwrap()
        .or(&x4.not().unwrap().and(&p).unwrap())
        .unwrap();

    mref.with_manager_shared(|manager| {
        visualize(
            manager,
            "test",
            &[&x0, &x1, &x2, &x3, &x4],
            Some(&["x0", "x1", "x2", "x3", "x4"]),
            &[&f],
            None,
            None,
        );
    })
}
