use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::{self, Write};

use fixedbitset::FixedBitSet;

use oxidd_core::function::{Function, INodeOfFunc, TermOfFunc};
use oxidd_core::util::{Borrowed, EdgeHashMap, EdgeVecDropGuard};
use oxidd_core::{Edge, HasLevel, InnerNode, LevelNo, Manager, Node};

use crate::AsciiDisplay;

use super::{Code, VarInfo};

// spell-checker:ignore varinfo,suppvar,varnames,suppvarnames,orderedvarnames
// spell-checker:ignore permids,auxids,rootids,rootnames
// spell-checker:ignore nnodes,nvars,nsuppvars,nroots

type FxBuildHasher = std::hash::BuildHasherDefault<rustc_hash::FxHasher>;

#[inline]
fn is_complemented<E: Edge>(edge: &E) -> bool {
    edge.tag() != Default::default()
}

/// DDDMP format version version
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, oxidd_derive::Countable, Debug,
)]
#[repr(u8)]
#[non_exhaustive]
pub enum DDDMPVersion {
    /// Version 2.0, bundled with [CUDD] 3.0
    ///
    /// [CUDD]: https://github.com/cuddorg/cudd
    #[default]
    V2_0,
    /// Version 3.0, used by [BDDSampler] and [Logic2BDD]
    ///
    /// [BDDSampler]: https://github.com/davidfa71/BDDSampler
    /// [Logic2BDD]: https://github.com/davidfa71/Extending-Logic
    V3_0,
}

impl fmt::Display for DDDMPVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DDDMPVersion::V2_0 => "DDDMP-2.0",
            DDDMPVersion::V3_0 => "DDDMP-3.0",
        })
    }
}

/// Settings for the DDDMP export
#[derive(Clone, Copy, Debug)]
pub struct ExportSettings<'a> {
    version: DDDMPVersion,
    ascii: bool,
    strict: bool,
    diagram_name: &'a str,
}

impl Default for ExportSettings<'_> {
    fn default() -> Self {
        Self {
            version: DDDMPVersion::default(),
            ascii: false,
            strict: true,
            diagram_name: "",
        }
    }
}

impl<'a> ExportSettings<'a> {
    /// Set the DDDMP format version for the export. Defaults to 2.0.
    #[inline(always)]
    pub fn version(mut self, version: DDDMPVersion) -> Self {
        self.version = version;
        self
    }
    /// Get the currently selected version
    #[inline(always)]
    pub fn get_version(&self) -> DDDMPVersion {
        self.version
    }

    /// Check if binary mode is supported for the decision diagram kind given by
    /// `F`
    #[inline(always)]
    pub fn binary_supported<M: Manager>(manager: &M) -> bool {
        // FIXME: `manager.num_terminals()` only represents the current terminal
        // node count, but we would like to know if it is always 1.
        M::InnerNode::ARITY == 2 && manager.num_terminals() == 1
    }
    /// Enforce ASCII mode. Nodes will be written in a human-readable way.
    ///
    /// By default, binary mode is used if supported.
    /// [supported][Self::binary_supported()].
    #[inline(always)]
    pub fn ascii(mut self) -> Self {
        self.ascii = true;
        self
    }
    /// Write nodes in the more compact binary mode if
    /// [supported][Self::binary_supported()]. This is the default.
    #[inline(always)]
    pub fn binary(mut self) -> Self {
        self.ascii = false;
        self
    }
    /// Get whether ASCII mode is enforced or binary mode will be used if
    /// [supported][Self::binary_supported()].
    #[inline(always)]
    pub fn is_ascii(&self) -> bool {
        self.ascii
    }

    /// Set the decision diagrams's name
    ///
    /// This corresponds to the `.dd` field in DDDMP. `name` should not contain
    /// any ASCII control characters (e.g., line breaks). Control characters
    /// will be replaced by spaces and in [strict mode][Self::strict()], an
    /// error will be generated during the export.
    #[inline(always)]
    pub fn diagram_name(mut self, name: &'a str) -> Self {
        self.diagram_name = name;
        self
    }
    /// Getter for [`Self::diagram_name()`]
    #[inline(always)]
    pub fn get_diagram_name(&self) -> &'a str {
        self.diagram_name
    }

    /// Enable or disable strict mode
    ///
    /// The DDDMP format imposes some restrictions on diagram, variable, and
    /// function names:
    ///
    /// - None of them may contain ASCII control characters (e.g., line breaks).
    /// - Variable and function names must not contain spaces either.
    /// - Variable and function names must not be empty. (However, it is
    ///   possible to not export any variable or function names at all.)
    ///
    /// In the diagram name, control characters will be replaced by spaces. In
    /// variable and function names, an underscore (`_`) is used as the
    /// replacement character. When using [`Self::export_with_names()`], empty
    /// function names are replaced by `_f{i}`, where `{i}` stands for the
    /// position in the iterator. Empty variable names are replaced by `_x{i}`.
    /// To retain uniqueness, as many underscores are added to the prefix as
    /// there are in the longest prefix over all present variable names.
    ///
    /// However, since such relabelling may lead to unexpected results, there is
    /// a strict mode. In strict mode, no variable names will be exported unless
    /// all variables are named. Additionally, an error will be generated upon
    /// any replacement. The error will not be propagated immediately but only
    /// after the file was written. This should simplify inspecting the error
    /// and also serve as a checkpoint for very long-running computations.
    ///
    /// By default, strict mode is enabled.
    #[inline(always)]
    pub fn strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }
    /// Getter for [`Self::strict()`]
    #[inline(always)]
    pub fn is_strict(&self) -> bool {
        self.strict
    }

    /// Export the decision diagram to `file`
    ///
    /// `functions` is an iterator over (references to) [`Function`]s. All nodes
    /// reachable from the root nodes of these functions will be included in the
    /// dump.
    ///
    /// Use [`Self::export_with_names()`] if you wish to assign names to the
    /// functions.
    ///
    /// Returns an error in case of an I/O failure or if
    /// [strict mode][Self::strict()] is enabled and the diagram name or a
    /// variable name does not meet the [requirements][Self::strict()]. In case
    /// one of the strict mode requirements is violated, the implementation
    /// attempts to complete the export before reporting the error. This is to
    /// help inspecting the error and also to save a checkpoint for very
    /// long-running computations.
    ///
    /// # Example
    ///
    /// ```
    /// # use oxidd_core::function::{self, Function};
    /// # use oxidd_dump::dddmp::ExportSettings;
    /// # fn export<F: Function>(f: &F, g: &F, h: &F) -> std::io::Result<()>
    /// # where
    /// #    for<'id> function::INodeOfFunc<'id, F>: oxidd_core::HasLevel,
    /// #    for<'id> function::TermOfFunc<'id, F>: oxidd_dump::AsciiDisplay,
    /// # {
    /// let file = std::fs::File::create("foo.dddmp")?;
    /// f.with_manager_shared(|manager, _| {
    ///     ExportSettings::default()
    ///         .diagram_name("foo")
    ///         .export(file, manager, [f, g, h])
    /// })?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn export<'id, FR: std::ops::Deref>(
        &self,
        file: impl io::Write,
        manager: &<FR::Target as Function>::Manager<'id>,
        functions: impl IntoIterator<Item = FR>,
    ) -> io::Result<()>
    where
        FR::Target: Function,
        INodeOfFunc<'id, FR::Target>: HasLevel,
        TermOfFunc<'id, FR::Target>: AsciiDisplay,
    {
        let iter = functions.into_iter();
        let mut roots = EdgeVecDropGuard::new(manager, Vec::with_capacity(iter.size_hint().0));
        roots.extend(iter.map(|f| manager.clone_edge(f.as_edge(manager))));

        export_common(file, self, manager, &roots, None, None)?;
        Ok(())
    }

    /// Export the decision diagram to `file_dddmp`, and node colors to `file_colors`
    ///
    /// `functions` is an iterator over (references to) [`Function`]s. All nodes
    /// reachable from the root nodes of these functions will be included in the
    /// dump.
    ///
    /// `colors` is an iterator over pairs of (references to) [`Function`]s
    /// and hex colors including a hashtag for coloring in the output diagram.
    ///
    /// Use [`Self::export_with_names()`] if you wish to assign names to the
    /// functions.
    ///
    /// Returns an error in case of an I/O failure or if
    /// [strict mode][Self::strict()] is enabled and the diagram name or a
    /// variable name does not meet the [requirements][Self::strict()]. In case
    /// one of the strict mode requirements is violated, the implementation
    /// attempts to complete the export before reporting the error. This is to
    /// help inspecting the error and also to save a checkpoint for very
    /// long-running computations.
    ///
    /// # Example
    ///
    /// ```
    /// # use oxidd_core::function::{self, Function};
    /// # use oxidd_dump::dddmp::ExportSettings;
    /// # fn export<F: Function>(f: &F, g: &F, h: &F) -> std::io::Result<()>
    /// # where
    /// #    for<'id> function::INodeOfFunc<'id, F>: oxidd_core::HasLevel,
    /// #    for<'id> function::TermOfFunc<'id, F>: oxidd_dump::AsciiDisplay,
    /// # {
    /// let file = std::fs::File::create("foo.dddmp")?;
    /// f.with_manager_shared(|manager, _| {
    ///     ExportSettings::default()
    ///         .diagram_name("foo")
    ///         .export(file, manager, [f, g, h])
    /// })?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn export_with_colors<'id, FR: std::ops::Deref, D: fmt::Display>(
        &self,
        file_dddmp: impl io::Write,
        mut file_colors: impl io::Write,
        manager: &<FR::Target as Function>::Manager<'id>,
        functions: impl IntoIterator<Item = FR>,
        colors: impl IntoIterator<Item = (FR, D)>,
    ) -> io::Result<()>
    where
        FR::Target: Function,
        INodeOfFunc<'id, FR::Target>: HasLevel,
        TermOfFunc<'id, FR::Target>: AsciiDisplay,
    {
        let root_iter = functions.into_iter();
        let mut roots = EdgeVecDropGuard::new(manager, Vec::with_capacity(root_iter.size_hint().0));
        roots.extend(root_iter.map(|f| manager.clone_edge(f.as_edge(manager))));

        let colors_iter = colors.into_iter();
        let mut colors_functions =
            EdgeVecDropGuard::new(manager, Vec::with_capacity(colors_iter.size_hint().0));
        let mut colors_hexes = Vec::<String>::with_capacity(1024);

        let mut res = Ok(());

        for (func, color) in colors_iter {
            colors_functions.push(manager.clone_edge(func.as_edge(manager)));

            let color = format!("{color}");
            if !is_valid_hex_color(&color) && self.strict && res.is_ok() {
                res = Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "colors must be valid hex colors of the form: #RRGGBB",
                ));
            }
            colors_hexes.push(color);
        }

        let ids = export_common(
            file_dddmp,
            self,
            manager,
            &roots,
            None,
            Some(&colors_functions),
        )?;
        for (id, color) in ids.iter().zip(colors_hexes) {
            if let Some(id) = id {
                writeln!(file_colors, "{id} {color}")?;
            }
        }

        res
    }

    /// Create a new export with function names
    ///
    /// `functions` is an iterator over pairs of (references to) [`Function`]s
    /// and names. All nodes reachable from the root nodes of these functions
    /// will be included in the dump.
    ///
    /// All functions names should be non-empty strings without spaces or ASCII
    /// control characters (e.g., newlines) due to requirements of the DDDMP
    /// format. Each space or ASCII control character will be replaced by an
    /// underscore (`_`). Empty names will be replaced by `_f{i}`. If
    /// [strict mode][Self::strict()] is enabled, a replacement will be reported
    /// as an error during export.
    ///
    /// Returns an error in case of an I/O failure or if
    /// [strict mode][Self::strict()] is enabled and the diagram name, a
    /// function name or a variable name does not meet the
    /// [requirements][Self::strict()]. In case  one of the strict mode
    /// requirements is violated, the implementation attempts to complete
    /// the export before reporting the error. This is to help inspecting
    /// the error and also to save a checkpoint for very long-running
    /// computations.
    ///
    /// # Example
    ///
    /// ```
    /// # use oxidd_core::function::{self, Function};
    /// # use oxidd_dump::dddmp::ExportSettings;
    /// # fn export<F: Function>(f: &F, g: &F, h: &F) -> std::io::Result<()>
    /// # where
    /// #    for<'id> function::INodeOfFunc<'id, F>: oxidd_core::HasLevel,
    /// #    for<'id> function::TermOfFunc<'id, F>: oxidd_dump::AsciiDisplay,
    /// # {
    /// let file = std::fs::File::create("foo.dddmp")?;
    /// f.with_manager_shared(|manager, _| {
    ///     ExportSettings::default()
    ///         .diagram_name("foo")
    ///         .export_with_names(file, manager, [(f, "f"), (g, "g"), (h, "h")])
    /// })?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn export_with_names<'id, FR: std::ops::Deref, D: fmt::Display>(
        &self,
        file: impl io::Write,
        manager: &<FR::Target as Function>::Manager<'id>,
        functions: impl IntoIterator<Item = (FR, D)>,
    ) -> io::Result<()>
    where
        FR::Target: Function,
        INodeOfFunc<'id, FR::Target>: HasLevel,
        TermOfFunc<'id, FR::Target>: AsciiDisplay,
    {
        let iter = functions.into_iter();
        let mut roots = EdgeVecDropGuard::new(manager, Vec::with_capacity(iter.size_hint().0));
        let mut root_names = Vec::<u8>::with_capacity(1024);

        let mut res = Ok(());

        for (i, (func, name)) in iter.enumerate() {
            roots.push(manager.clone_edge(func.as_edge(manager)));

            root_names.push(b' ');
            let len_before = root_names.len();
            write!(&mut root_names, "{name}")?;

            let name_bytes = &mut root_names[len_before..];
            if name_bytes.is_empty() {
                write!(&mut root_names, "_f{i}")?;
                if self.strict && res.is_ok() {
                    res = Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "function names must not be empty",
                    ));
                }
                continue;
            }

            for c in name_bytes {
                if c.is_ascii_control() || *c == b' ' {
                    *c = b'_';
                    if self.strict && res.is_ok() {
                        res = Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "function names must not contain control characters or spaces",
                        ));
                    }
                }
            }
        }

        export_common(file, self, manager, &roots, Some(&root_names), None)?;
        res
    }
}

fn export_common<M: Manager, W: io::Write>(
    mut file_dddmp: W,
    settings: &ExportSettings,
    manager: &M,
    roots: &[M::Edge],
    root_names: Option<&[u8]>,
    func_ids: Option<&[M::Edge]>,
) -> io::Result<Vec<Option<usize>>>
where
    M::InnerNode: HasLevel,
    M::Terminal: crate::AsciiDisplay,
{
    writeln!(file_dddmp, ".ver {}", settings.version)?;
    let ascii = settings.ascii || !ExportSettings::binary_supported(manager);
    writeln!(file_dddmp, ".mode {}", if ascii { 'A' } else { 'B' })?;

    // TODO: other .varinfo modes?
    writeln!(file_dddmp, ".varinfo {}", VarInfo::None as u32)?;

    let mut res = Ok(());

    if !settings.diagram_name.is_empty() {
        write!(file_dddmp, ".dd ")?;
        if write_replacing_control(&mut file_dddmp, settings.diagram_name)? && settings.strict {
            res = Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "decision diagram name must not contain control characters",
            ));
        }
        writeln!(file_dddmp)?;
    }

    let nvars = manager.num_levels();

    // Map from the current level number to its internal var index (almost the
    // same numbering, but vars not in the support removed, i.e., range
    // `0..nsuppvars`), and the nodes (as edges) contained at this level
    // together with their indexes.
    let mut node_map: Vec<(LevelNo, _)> =
        (0..nvars).map(|_| (0, EdgeHashMap::new(manager))).collect();
    let mut terminal_map = EdgeHashMap::new(manager);

    fn rec_add_map<M: Manager>(
        manager: &M,
        node_map: &mut Vec<(u32, EdgeHashMap<M, usize, FxBuildHasher>)>,
        terminal_map: &mut EdgeHashMap<M, usize, FxBuildHasher>,
        e: Borrowed<M::Edge>,
    ) where
        M::InnerNode: HasLevel,
    {
        match manager.get_node(&e) {
            Node::Inner(node) => {
                let (_, map) = &mut node_map[node.level() as usize];
                // Map to 0 -> we assign the indexes below
                let res = map.insert(&e.with_tag(Default::default()), 0);
                if res.is_none() {
                    for e in node.children() {
                        rec_add_map::<M>(manager, node_map, terminal_map, e);
                    }
                }
            }
            Node::Terminal(_) => {
                terminal_map.insert(&e.with_tag(Default::default()), 0);
            }
        }
    }

    for root in roots.iter() {
        rec_add_map(manager, &mut node_map, &mut terminal_map, root.borrowed());
    }

    let mut nnodes = 0;
    for (_, idx) in &mut terminal_map {
        nnodes += 1; // pre increment -> numbers in range 1..=#nodes
        *idx = nnodes;
    }
    let nsuppvars = node_map.iter().filter(|(_, m)| !m.is_empty()).count() as u32;
    let mut supp_levels = FixedBitSet::with_capacity(nvars as usize);
    let mut suppvar_idx = nsuppvars;
    // rev() -> assign node IDs bottom-up
    for (i, (var_idx, level)) in node_map.iter_mut().enumerate().rev() {
        if level.is_empty() {
            continue;
        }
        suppvar_idx -= 1;
        *var_idx = suppvar_idx;
        supp_levels.insert(i);
        for (_, idx) in level.iter_mut() {
            nnodes += 1; // pre increment -> numbers in range 1..=#nodes
            *idx = nnodes;
        }
    }
    writeln!(file_dddmp, ".nnodes {nnodes}")?;
    writeln!(file_dddmp, ".nvars {nvars}")?;
    writeln!(file_dddmp, ".nsuppvars {nsuppvars}")?;

    if manager.num_named_vars() == nvars || (!settings.strict && manager.num_named_vars() != 0) {
        let mut leading_underscores = 1;
        let mut replaced = 0usize;
        let var_names: Vec<Cow<str>> = Vec::from_iter((0..nvars).map(|i| {
            let name = replace_space_and_control(manager.var_name(i));
            if let Cow::Owned(_) = &name {
                replaced += 1;
            }
            for (i, b) in name.bytes().enumerate() {
                if b != b'_' {
                    break;
                }
                leading_underscores = i + 2;
            }
            name
        }));

        let mut prefix_all_owned = false;
        if replaced != 0 {
            let mut replaced_set: HashSet<&str, FxBuildHasher> =
                HashSet::with_capacity_and_hasher(replaced, FxBuildHasher::default());
            for name in var_names.iter() {
                if let Cow::Owned(name) = &name {
                    if manager.name_to_var(name).is_some() || !replaced_set.insert(name) {
                        prefix_all_owned = true;
                        break;
                    }
                }
            }

            if settings.strict && res.is_ok() {
                res = Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "variable names must not contain control characters or spaces",
                ));
            }
        }

        let write_var = |file: &mut W, i: usize| match &var_names[i] {
            Cow::Borrowed("") => write!(file, " {:_<leading_underscores$}x{i}", ""),
            Cow::Borrowed(name) => write!(file, " {name}"),
            Cow::Owned(name) if !prefix_all_owned => write!(file, " {name}"),
            Cow::Owned(name) => write!(file, " {:_<leading_underscores$}x{i}_{name}", ""),
        };

        if let DDDMPVersion::V3_0 = settings.version {
            write!(file_dddmp, ".varnames")?;
            for var in 0..nvars {
                write_var(&mut file_dddmp, var as usize)?;
            }
            writeln!(file_dddmp)?;
        }

        write!(file_dddmp, ".suppvarnames")?;
        for var in 0..nvars {
            if supp_levels.contains(manager.var_to_level(var) as usize) {
                write_var(&mut file_dddmp, var as usize)?;
            }
        }
        writeln!(file_dddmp)?;

        write!(file_dddmp, ".orderedvarnames")?;
        for level in 0..nvars {
            write_var(&mut file_dddmp, manager.level_to_var(level) as usize)?;
        }
        writeln!(file_dddmp)?;
    }

    write!(file_dddmp, ".ids")?;
    for id in 0..nvars {
        if supp_levels.contains(manager.var_to_level(id) as usize) {
            write!(file_dddmp, " {id}")?;
        }
    }
    writeln!(file_dddmp)?;

    write!(file_dddmp, ".permids")?;
    for var in 0..nvars {
        let level = manager.var_to_level(var);
        if supp_levels.contains(level as usize) {
            write!(file_dddmp, " {level}")?;
        }
    }
    writeln!(file_dddmp)?;

    // TODO: .auxids?

    let idx = |e: &M::Edge| {
        let idx = match manager.get_node(e) {
            Node::Inner(node) => {
                let (_, map) = &node_map[node.level() as usize];
                *map.get(&e.with_tag(Default::default())).unwrap() as isize
            }
            Node::Terminal(_) => {
                *terminal_map.get(&e.with_tag(Default::default())).unwrap() as isize
            }
        };
        if is_complemented(e) {
            -idx
        } else {
            idx
        }
    };
    let bin_idx = |e: &M::Edge, node_id: usize| {
        let idx = match manager.get_node(e) {
            Node::Inner(node) => {
                let (_, map) = &node_map[node.level() as usize];
                *map.get(&e.with_tag(Default::default())).unwrap()
            }
            // TODO: How to support multiple terminals?
            Node::Terminal(_) => return (Code::Terminal, 0),
        };
        if idx == node_id - 1 {
            (Code::Relative1, 0)
        } else if node_id - idx < idx {
            (Code::RelativeID, node_id - idx)
        } else {
            (Code::AbsoluteID, idx)
        }
    };

    writeln!(file_dddmp, ".nroots {}", roots.len())?;
    write!(file_dddmp, ".rootids")?;
    for root in roots {
        write!(file_dddmp, " {}", idx(root))?;
    }
    writeln!(file_dddmp)?;
    if let Some(root_names) = root_names {
        write!(file_dddmp, ".rootnames")?;
        file_dddmp.write_all(root_names)?;
        writeln!(file_dddmp)?;
    }

    writeln!(file_dddmp, ".nodes")?;

    #[inline]
    const fn node_code(var: Code, t: Code, e_complement: bool, e: Code) -> u8 {
        ((var as u8) << 5) | ((t as u8) << 3) | ((e_complement as u8) << 2) | e as u8
    }

    let mut out_ids: Vec<Option<usize>> = (0..func_ids.map(|f| f.len()).unwrap_or_default())
        .map(|_| None)
        .collect();
    let out_index_map = func_ids
        .map(|f| {
            f.iter()
                .enumerate()
                .map(|(id, e)| (e, id))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();

    let mut exported_nodes = 0;
    for (edge, &node_id) in terminal_map.iter() {
        // This implementation relies on that the iteration order for
        // `EdgeHashMap`s stays the same as long as no elements are
        // inserted/removed. To be sure, we assert that this assumption holds.
        assert_eq!(exported_nodes + 1, node_id);
        if ascii {
            // <Node-index> [<Var-extra-info>] <Var-internal-index> <Then-index>
            // <Else-index>
            let node = manager.get_node(edge);
            let desc = Ascii(node.unwrap_terminal());
            writeln!(file_dddmp, "{node_id} {desc} 0 0")?;
            if let Some(&index) = out_index_map.get(edge) {
                out_ids[index] = Some(node_id);
            }
        } else {
            let terminal = node_code(Code::Terminal, Code::Terminal, false, Code::Terminal);
            write_escaped(&mut file_dddmp, &[terminal])?;
        }
        exported_nodes += 1;
    }
    // Work from bottom to top
    for &(var_idx, ref level) in node_map.iter().rev() {
        for (e, &node_id) in level.iter() {
            if let Some(&index) = out_index_map.get(e) {
                out_ids[index] = Some(node_id);
            }
            assert_eq!(exported_nodes + 1, node_id);
            let node = manager.get_node(e).unwrap_inner();
            if ascii {
                // <Node-index> [<Var-extra-info>] <Var-internal-index> <Then-index>
                // <Else-index>
                write!(file_dddmp, "{node_id} {var_idx}")?;
                for child in node.children() {
                    write!(file_dddmp, " {}", idx(&child))?;
                }
                writeln!(file_dddmp)?;
            } else {
                let mut iter = node.children();
                let t = iter.next().unwrap();
                let t_lvl = manager.get_node(&t).level();
                let e = iter.next().unwrap();
                let e_lvl = manager.get_node(&e).level();
                debug_assert!(iter.next().is_none());

                let mut var_code = Code::AbsoluteID;
                let mut var_idx = var_idx;
                let min_lvl = std::cmp::min(t_lvl, e_lvl);
                if min_lvl != LevelNo::MAX {
                    let (min_var_idx, _) = node_map[min_lvl as usize];
                    if var_idx == min_var_idx - 1 {
                        var_code = Code::Relative1;
                    } else if min_var_idx - var_idx < var_idx {
                        var_code = Code::RelativeID;
                        var_idx = min_var_idx - var_idx;
                    }
                }

                let (t_code, t_idx) = bin_idx(&t, node_id);
                let (e_code, e_idx) = bin_idx(&e, node_id);

                debug_assert!(!is_complemented(&*t));
                write_escaped(
                    &mut file_dddmp,
                    &[node_code(var_code, t_code, is_complemented(&*e), e_code)],
                )?;
                if var_code == Code::AbsoluteID || var_code == Code::RelativeID {
                    encode_7bit(&mut file_dddmp, var_idx as usize)?;
                }
                if t_code == Code::AbsoluteID || t_code == Code::RelativeID {
                    encode_7bit(&mut file_dddmp, t_idx)?;
                }
                if e_code == Code::AbsoluteID || e_code == Code::RelativeID {
                    encode_7bit(&mut file_dddmp, e_idx)?;
                }
            }
            exported_nodes += 1;
        }
    }

    writeln!(file_dddmp, ".end")?;

    res?;
    Ok(out_ids)
}

struct Ascii<T>(T);
impl<T: crate::AsciiDisplay> fmt::Display for Ascii<&T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Returns whether an ASCII control character got replaced by a space
fn write_replacing_control(mut writer: impl io::Write, string: &str) -> io::Result<bool> {
    let mut bytes = string.as_bytes();
    let mut did_replace = false;
    'outer: loop {
        for (i, &b) in bytes.iter().enumerate() {
            if b.is_ascii_control() {
                did_replace = true;
                writer.write_all(&bytes[..i])?;
                writer.write_all(b" ")?;
                bytes = &bytes[i + 1..];
                continue 'outer;
            }
        }

        writer.write_all(bytes)?;
        return Ok(did_replace);
    }
}

fn replace_space_and_control(mut string: &str) -> Cow<'_, str> {
    let mut result = Cow::Borrowed(string);
    'outer: loop {
        for (i, b) in string.bytes().enumerate() {
            if b.is_ascii_control() {
                if let Cow::Borrowed(_) = result {
                    result = Cow::Owned(String::with_capacity(string.len()));
                }
                let owned = result.to_mut();
                let (pre, post_with_b) = string.split_at(i);
                owned.push_str(pre);
                owned.push('_');
                string = post_with_b.split_at(1).1;
                continue 'outer;
            }
        }

        if let Cow::Owned(owned) = &mut result {
            owned.push_str(string);
        }
        return result;
    }
}

/// 7-bit encode an integer and write it (escaped) via `writer`
fn encode_7bit(writer: impl io::Write, mut value: usize) -> io::Result<()> {
    let mut buf = [0u8; (usize::BITS as usize + 8 - 1) / 7];
    let mut idx = buf.len() - 1;
    buf[idx] = (value as u8).wrapping_shl(1);
    value >>= 7;
    while value != 0 {
        idx -= 1;
        buf[idx] = (value as u8).wrapping_shl(1) | 1;
        value >>= 7;
    }
    write_escaped(writer, &buf[idx..])
}

/// Write an DDDMP-style escaped byte
fn write_escaped(mut writer: impl io::Write, buf: &[u8]) -> io::Result<()> {
    for c in buf {
        writer.write_all(match *c {
            0x00 => &[0x00, 0x00],
            0x0a => &[0x00, 0x01],
            0x0d => &[0x00, 0x02],
            0x1a => &[0x00, 0x03],
            _ => std::slice::from_ref(c),
        })?;
    }
    Ok(())
}

fn is_valid_hex_color(s: &str) -> bool {
    let chars = s.as_bytes();

    if chars.len() != 7 || chars[0] != b'#' {
        return false;
    }

    chars[1..].iter().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_encode_7bit() -> io::Result<()> {
        let mut buf: Vec<u8> = Vec::new();

        if usize::BITS == 64 {
            encode_7bit(&mut buf, usize::MAX)?;
            assert_eq!(
                &buf[..],
                &[
                    0b0000_0011, //  1
                    0b1111_1111, //  8
                    0b1111_1111, // 15
                    0b1111_1111, // 22
                    0b1111_1111, // 29
                    0b1111_1111, // 36
                    0b1111_1111, // 43
                    0b1111_1111, // 50
                    0b1111_1111, // 57
                    0b1111_1110, // 64
                ]
            );
        }

        buf.clear();
        encode_7bit(&mut buf, 0)?;
        assert_eq!(&buf[..], &[0, 0]); // 2 times 0 due to escaping

        buf.clear();
        encode_7bit(&mut buf, 1)?;
        assert_eq!(&buf[..], &[0b10]);

        Ok(())
    }
}
