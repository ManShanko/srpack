/// Attempt at a zero alloc commandline parsing library.
///
/// Through use of const generics and declarative macros the library
/// can construct a union object to hold any expected subcommand/switch.
///
/// The interface currently has a couple of gotchas:
///
/// 1.  An error in a macro can spew larges amounts of errors during
///     compile due to error propagation in const variable usage.
///
/// 2.  Large amounts of `Switch`es or `CommandBuilder`s will cause the
///     rustc const interpreter to fail during compile.
///
/// 3.  The use of const requires explicit type annotation in declarations
///     and only allows `'static` references for types like `str`.
use core::array;
use core::str;
use std::env;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::io;
use std::io::Write;
use std::path::Path;

// 32 spaces to pad for Switch::description offset
const PADDING: &'static str = "                                ";

#[doc(hidden)]
pub type SwitchInner = (Option<&'static str>, usize, usize, usize);

#[doc(hidden)]
#[macro_export]
macro_rules! _one_internal {
    ($ignore:expr) => {1}
}

#[macro_export]
macro_rules! command {
     ( $( $switch:expr, )* ) => {
        {
            // TODO: find way to avoid inserting Switch.short when short.is_none()
            const SORTED: [crate::cli::SwitchInner; 0 $( + (2 * _one_internal!($switch)) )*]
                = crate::cli::insertion_sort_switch({
                    let mut _count = 0;
                    let mut _offset = 0;
                    [
                        $(
                            {
                                let (short, _) = $switch.switches();
                                (short, _count, _offset, $switch.num_params())
                            },
                            {
                                let (_, long) = $switch.switches();
                                let c = _count;
                                _count += 1;
                                let off = _offset;
                                _offset += $switch.num_params();
                                (Some(long), c, off, $switch.num_params())
                            },
                        )*
                    ]
                });

            const OUT: crate::cli::CommandBuilder = crate::cli::CommandBuilder::new(&[
                $( &$switch, )*
            ], &SORTED);
            OUT
        }
     }
}

#[macro_export]
macro_rules! app {
    (
        $( $cmd:expr, )*
    ) => {
        {
            const NUM_COMMANDS: usize = 0
                $( + _one_internal!($cmd) )?
                ;
            const MAX_PARAMETERS: usize = crate::cli::count_array(&[
                $( $cmd.num_params(), )*
            ]);
            const MAX_UNIQUE_SWITCHES: usize = crate::cli::count_array(&[
                $( $cmd.num_unique_switches(), )*
            ]);
            const MAX_SWITCHES: usize = crate::cli::count_array(&[
                $( $cmd.num_switches(), )*
            ]);

            debug_assert!(MAX_UNIQUE_SWITCHES <= MAX_SWITCHES);

            const MAX_SWITCH_PARAMETERS: usize = crate::cli::count_array(&[
                $( $cmd.num_switch_params(), )*
            ]);
            const SUBCOMMANDS: crate::cli::SortedCommandBuilder<NUM_COMMANDS> =
                crate::cli::insertion_sort_command_builder([
                    $( ($cmd.name(), &$cmd), )*
                ]);

            let cli = crate::cli::Cli::new();
            cli.process_args_os_to_stdout::<NUM_COMMANDS, MAX_PARAMETERS, MAX_UNIQUE_SWITCHES, MAX_SWITCHES, MAX_SWITCH_PARAMETERS>(
                SUBCOMMANDS,
                ::std::env::args_os(),
                &mut ::std::io::stdout(),
            )
        }
    }
}

pub struct SortedCommandBuilder<const N: usize>(
    [(Option<&'static str>, &'static CommandBuilder); N],
);

// based on Ord implementation for str
const fn compare_optional_str_(a: Option<&'static str>, b: Option<&'static str>, reverse_none: bool) -> isize {
    if let Some(a) = a
        && let Some(b) = b
    {
        let a = a.as_bytes();
        let b = b.as_bytes();

        let len = if a.len() < b.len() {
            a.len()
        } else {
            b.len()
        };

        let mut i: usize = 0;
        while i < len {
            let lhs = a[i];
            let rhs = b[i];
            if lhs > rhs {
                return 1;
            } else if lhs < rhs {
                return -1;
            }
            i += 1;
        }

        if a.len() > b.len() {
            1
        } else if a.len() < b.len() {
            -1
        } else {
            0
        }
    } else {
        if reverse_none {
            if a.is_some() {
                -1
            } else if b.is_some() {
                1
            } else /* a.is_none() && b.is_none() */ {
                0
            }
        } else {
            if a.is_some() {
                1
            } else if b.is_some() {
                -1
            } else /* a.is_none() && b.is_none() */ {
                0
            }
        }
    }
}

const fn compare_optional_str(a: Option<&'static str>, b: Option<&'static str>) -> isize {
    compare_optional_str_(a, b, false)
}

const fn compare_optional_str_2(a: Option<&'static str>, b: Option<&'static str>) -> isize {
    compare_optional_str_(a, b, true)
}

#[doc(hidden)]
pub const fn insertion_sort_switch<const N: usize>(
    mut arr: [SwitchInner; N],
) -> [SwitchInner; N] {
    let mut i = 0;
    while i < arr.len() {
        let mut j = i;
        while j > 0 && 1 == compare_optional_str_2(arr[j-1].0, arr[j].0) {
            let a = arr[j - 1];
            let b = arr[j];
            arr[j] = a;
            arr[j - 1] = b;
            j -= 1;
        }
        i += 1;
    }
    arr
}

#[doc(hidden)]
pub const fn insertion_sort_command_builder<const N: usize>(
    mut arr: [(Option<&'static str>, &'static CommandBuilder); N],
) -> SortedCommandBuilder<N> {
    let mut i = 0;
    while i < arr.len() {
        let mut j = i;
        while j > 0 && 1 == compare_optional_str(arr[j-1].0, arr[j].0) {
            let a = arr[j - 1];
            let b = arr[j];
            arr[j] = a;
            arr[j - 1] = b;
            j -= 1;
        }
        i += 1;
    }
    SortedCommandBuilder(arr)
}

#[doc(hidden)]
pub const fn count_array(arr: &[usize]) -> usize {
    let mut out = 0;
    let mut i = 0;
    while i < arr.len() {
        if arr[i] > out {
            out = arr[i];
        }
        i += 1;
    }
    out
}

enum SwitchLength {
    Short,
    Long,
}

fn osstr_switch_kind(os: &OsStr) -> Option<SwitchLength> {
    let mut iter;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        iter = os.encode_wide();
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::ffi::OsStrExt;
        iter = os.as_bytes().iter().map(|b| *b);
    }

    if let Some(w0) = iter.next()
        && w0 == b'-'.into()
        && let Some(w1) = iter.next()
    {
        if w1 == b'-'.into()
        {
            return Some(SwitchLength::Long);
        }
        return Some(SwitchLength::Short);
    }
    None
}

pub struct Switch {
    short: Option<&'static str>,
    long: &'static str,
    desc: Option<&'static str>,
    params: Option<&'static [&'static str]>,
}

impl Switch {
    const fn new_(short: Option<&'static str>, long: &'static str) -> Switch {
        if let Some(s) = short {
            let s = s.as_bytes();
            debug_assert!(s[0] < 128 && s.len() == 1);
        }
        let l = long.as_bytes();
        debug_assert!(l.len() >= 2);

        Self {
            short,
            long,
            params: None,
            desc: None,
        }
    }

    pub const fn new(long: &'static str) -> Switch {
        Self::new_(None, long)
    }

    pub const fn short(short: &'static str, long: &'static str) -> Switch {
        Self::new_(Some(short), long)
    }

    pub const fn with_params(mut self, params: &'static [&'static str]) -> Self {
        self.params = Some(params);
        self
    }

    pub const fn with_desc(mut self, desc: &'static str) -> Self {
        self.desc = Some(desc);
        self
    }

    pub fn description(&self) -> Option<&str> {
        self.desc
    }

    pub const fn num_params(&self) -> usize {
        if let Some(params) = self.params {
            params.len()
        } else {
            0
        }
    }

    pub const fn switches(&self) -> (Option<&'static str>, &'static str) {
        (self.short, self.long)
    }

    pub const fn num_switches(&self) -> usize {
        let mut count = 1;
        if self.short.is_some() {
            count += 1;
        }
        count
    }

    fn help(&self, pipe: &mut dyn Write) -> io::Result<()> {
        let long = self.long;
        let mut size = match self.short {
            Some(short) => {
                write!(pipe, "    -{short}, --{long}")?;
                b"    -".len() + 1 + b", --".len() + long.len()
            }
            None => {
                write!(pipe, "        --{long}")?;
                b"        --".len() + long.len()
            }
        };

        if let Some(params) = self.params {
            if params.len() > 1 {
                write!(pipe, " [...]")?;
                size += " [...]".len();
            } else if let Some(param) = params.get(0) {
                write!(pipe, " <{param}>")?;
                size += " <>".len() + param.len();
            }
        }

        let offset = 32_usize.saturating_sub(size).max(1);

        match self.desc {
            Some(desc) => writeln!(pipe, "{}{}", &PADDING[..offset], desc)?,
            None => writeln!(pipe)?,
        }

        Ok(())
    }
}

impl AsRef<str> for Switch {
    fn as_ref(&self) -> &'static str {
        self.long
    }
}

pub struct CommandBuilder {
    name: Option<&'static str>,
    switches: &'static [&'static Switch],
    sorted_switches: &'static [SwitchInner],
    num_switch_params: usize,
    params: Option<&'static [&'static str]>,
    desc: Option<&'static str>,
    short_desc: Option<&'static str>,
}

impl CommandBuilder {
    pub const fn new(
        switches: &'static [&'static Switch],
        sorted_switches: &'static [SwitchInner],
    ) -> CommandBuilder {
        let mut num_switch_params = 0;
        let mut i = 0;
        while i < switches.len() {
            num_switch_params += switches[i].num_params();
            i += 1;
        }

        CommandBuilder {
            name: None,
            switches,
            sorted_switches,
            num_switch_params,
            params: None,
            desc: None,
            short_desc: None,
        }
    }

    pub const fn with_name(mut self, name: &'static str) -> Self {
        self.name = Some(name);
        self
    }

    pub const fn with_params(mut self, params: &'static [&'static str]) -> Self {
        self.params = Some(params);
        self
    }

    pub const fn with_desc(mut self, desc: &'static str) -> Self {
        self.desc = Some(desc);
        self
    }

    pub const fn with_short_desc(mut self, desc: &'static str) -> Self {
        self.short_desc = Some(desc);
        self
    }

    pub const fn name(&self) -> Option<&'static str> {
        self.name
    }

    pub const fn num_unique_switches(&self) -> usize {
        self.switches.len()
    }

    pub const fn num_switches(&self) -> usize {
        let mut count = 0;
        let mut i = 0;
        while i < self.switches.len() {
            count += self.switches[i].num_switches();
            i += 1;
        }
        count
    }

    pub const fn num_params(&self) -> usize {
        let mut out = 0;
        if let Some(p) = self.params {
            out = p.len()
        }
        out
    }

    pub const fn num_switch_params(&self) -> usize {
        self.num_switch_params
    }

    pub const fn switches(&self) -> &'static [&'static Switch] {
        self.switches
    }

    fn help(&self, pipe: &mut dyn Write) -> io::Result<()> {
        if let Some(desc) = self.desc.or(self.short_desc) {
            writeln!(pipe, "{desc}\n")?;
        }

        if let Some(name) = self.name {
            write!(pipe, "USAGE:\n    {} {}", Cli::NAME, name)?;
            if !self.switches.is_empty() {
                write!(pipe, " [SWITCHES]")?;
            }
            if let Some(params) = self.params {
                for param in params.iter() {
                    write!(pipe, " [{param}]")?;
                }
            }
            writeln!(pipe)?;
        }

        if !self.switches.is_empty() {
            if self.name.is_some() {
                writeln!(pipe)?;
            }
            writeln!(pipe, "SWITCHES:")?;
            for s in self.switches.iter() {
                s.help(pipe)?;
            }
        }

        pipe.flush()
    }
}

pub struct Params<'c>(&'c [Option<OsString>]);

impl<'c> Iterator for Params<'c> {
    type Item = &'c OsStr;

    fn next(&mut self) -> Option<Self::Item> {
        if self.0.len() > 0 {
            let out;
            (out, self.0) = self.0.split_at(1);
            out.get(0).and_then(|o| o.as_ref().map(|o| o.as_os_str()))
        } else {
            None
        }
    }
}

pub struct Command<
    const MAX_PARAMETERS: usize,
    const MAX_UNIQUE_SWITCHES: usize,
    const MAX_SWITCHES: usize,
    const MAX_SWITCH_PARAMETERS: usize,
> {
    command: &'static CommandBuilder,
    params: [Option<OsString>; MAX_PARAMETERS],
    switches: &'static [SwitchInner],
    switches_active: [bool; MAX_UNIQUE_SWITCHES],
    switches_params: [Option<OsString>; MAX_SWITCH_PARAMETERS],
    unused: usize,
}

impl<
    const MAX_PARAMETERS: usize,
    const MAX_UNIQUE_SWITCHES: usize,
    const MAX_SWITCHES: usize,
    const MAX_SWITCH_PARAMETERS: usize,
> Command<MAX_PARAMETERS, MAX_UNIQUE_SWITCHES, MAX_SWITCHES, MAX_SWITCH_PARAMETERS> {
    fn new(
        cmd: &'static CommandBuilder,
        switches: &'static [SwitchInner],
    ) -> Command<MAX_PARAMETERS, MAX_UNIQUE_SWITCHES, MAX_SWITCHES, MAX_SWITCH_PARAMETERS> {
        if cfg!(debug_assertions) {
            for (flag, _, _, _) in switches.iter() {
                assert!(flag.is_some());
            }
        }

        Command {
            command: cmd,
            params: array::from_fn(|_| None),
            switches,
            switches_active: array::from_fn(|_| false),
            switches_params: array::from_fn(|_| None),
            unused: 0,
        }
    }

    fn params_left(&mut self) -> usize {
        let mut count = 0;
        for param in self.params.iter() {
            if param.is_none() {
                break;
            }
            count += 1;
        }
        self.params.len() - count
    }

    fn insert_param(&mut self, param: OsString) {
        if let Some(slot) = self.params.iter_mut().find(|v| v.is_none()) {
            *slot = Some(param);
        }
    }

    fn switch_activate(&mut self, i: usize) {
        debug_assert!(self.switches.get(i).is_some());
        if let Some(&(_, active_index, _, _)) = self.switches.get(i) {
            self.switches_active[active_index] = true;
        }
    }

    fn switch_params_left(&self, i: usize) -> Option<usize> {
        debug_assert!(self.switches.get(i).is_some());
        if let Some(&(_, _, offset, num_params)) = self.switches.get(i) {
            let mut count = 0;
            for param in self.switches_params[offset..offset + num_params].iter() {
                if param.is_none() {
                    break;
                }
                count += 1
            }
            Some(num_params - count)
        } else {
            None
        }
    }

    fn insert_switch_param(&mut self, i: usize, param: OsString) {
        debug_assert!(self.switches.get(i).is_some());
        if let Some(&(_, _, offset, num_params)) = self.switches.get(i)
            && let Some(slot) = self.switches_params[offset..offset + num_params].iter_mut().find(|v| v.is_none())
        {
            *slot = Some(param);
        }
    }

    pub fn num_switches(&self) -> usize {
        self.switches.len()
    }

    pub fn num_parameters(&self) -> usize {
        self.switches.iter().map(|(_, _, _, nv)| *nv).sum()
    }

    pub fn params(&self) -> Params {
        Params(&self.params[..])
    }

    fn switch_active_(&self, switch: &str) -> bool {
        if let Ok(i) = self.switches.binary_search_by(|probe| probe.0.cmp(&Some(switch)))
            && let Some(&(_, active_index, _, _)) = self.switches.get(i)
            && {
                debug_assert!(self.switches_active.get(active_index).is_some(),
                    "{} {}", active_index, self.switches_active.len());
                true
            }
            && let Some(&active) = self.switches_active.get(active_index)
        {
            active
        } else {
            false
        }
    }

    pub fn switch_active<T: AsRef<str>>(&self, switch: T) -> bool {
        self.switch_active_(switch.as_ref())
    }

    fn switch_params_(&self, switch: &str) -> Option<Params> {
        if let Ok(i) = self.switches.binary_search_by(|probe| probe.0.cmp(&Some(switch)))
            && let Some(&(_, active_index, offset, num_params)) = self.switches.get(i)
            && let Some(true) = self.switches_active.get(active_index)
        {
            Some(Params(&self.switches_params[offset..offset + num_params]))
        } else {
            None
        }
    }

    pub fn switch_params<T: AsRef<str>>(&self, switch: T) -> Option<Params> {
        self.switch_params_(switch.as_ref())
    }

    pub fn unused_arguments(&self) -> usize {
        self.unused
    }

    pub fn command(&self) -> &'static CommandBuilder {
        self.command
    }

    pub fn name(&self) -> Option<&'static str> {
        self.command.name
    }

    pub fn subcmd(&self, builder: &CommandBuilder) -> bool {
        self.command as *const CommandBuilder == builder as *const CommandBuilder
    }
}

pub struct Cli;

impl Cli {
    const NAME: &'static str = env!("CARGO_PKG_NAME");
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    pub fn new() -> Cli {
        Cli
    }

    pub fn help(
        &self,
        cmds: &[(Option<&'static str>, &'static CommandBuilder)],
        pipe: &mut dyn Write,
    ) -> io::Result<()> {
        debug_assert!(!cmds.is_empty());
        debug_assert!(cmds.get(0).map(|(name, _)| name) == None || cmds.len() > 1);

        write!(
            pipe,
            concat!(
                "{name} - {version}\n",
                "\n",
                "USAGE:\n",
                "    {name}"
            ),
            name = Self::NAME,
            version = Self::VERSION
        )?;

        if let Some(cmd) = cmds.get(0)
            && cmd.0.is_none()
        {
            if !cmd.1.switches.is_empty() {
                write!(pipe, " [SWITCHES]")?;
            }
        }

        if cmds.len() > 1 {
            write!(pipe, " [SUBCOMMAND]")?;
        } else if let Some(cmd) = cmds.get(0) {
            //if let Some(name) = cmd.0 {
            //    write!(pipe, " {name}")?;
            //}

            if let Some(params) = cmd.1.params {
                for param in params {
                    write!(pipe, " [{param}]")?;
                }
            }
            writeln!(pipe)?;
        }

        writeln!(pipe)?;

        let mut iter = cmds.iter();
        let mut next_cmd = iter.next();

        if let Some(base) = next_cmd
            && base.0.is_none()
        {
            writeln!(pipe, "SWITCHES:")?;
            for s in base.1.switches.iter() {
                s.help(pipe)?;
            }
            next_cmd = iter.next();
        }

        if next_cmd.is_some() {
            writeln!(pipe, "\nCOMMANDS:")?;
            while let Some((name, cmd)) = next_cmd {
                debug_assert!(name.is_some(), "unexpected subcommand with None");
                if let Some(name) = name {
                    write!(pipe, "    {name}")?;
                    if let Some(desc) = cmd.short_desc {
                        let len = "    ".len() + name.len();
                        let len = 16_usize.saturating_sub(len + 1).max(2);
                        write!(pipe, "{}{desc}", &PADDING[..len])?;
                    }
                    writeln!(pipe)?;
                    next_cmd = iter.next();
                }
            }
        }

        Ok(())
    }

    pub fn process_args_os_to_stdout<
        const NUM_COMMANDS: usize,
        const MAX_PARAMETERS: usize,
        const MAX_UNIQUE_SWITCHES: usize,
        const MAX_SWITCHES: usize,
        const MAX_SWITCH_PARAMETERS: usize,
    >(
        &self,
        cmds: SortedCommandBuilder<NUM_COMMANDS>,
        mut args: env::ArgsOs,
        pipe: &mut dyn Write,
    ) -> io::Result<
        Option<Command<MAX_PARAMETERS, MAX_UNIQUE_SWITCHES, MAX_SWITCHES, MAX_SWITCH_PARAMETERS>>
    > {
        let cmds = cmds.0;
        if cfg!(debug_assertions) {
            cmds.iter().fold(None, |prev, (name, _)| {
                if let Some(prev) = prev {
                    debug_assert!(prev <= name);
                }
                Some(name)
            });
        }

        // assume first argument is executable path
        args.next();
        let mut next_arg = args.next();

        let cmd_index = if cmds.len() == 1 {
            if let Some(ref arg) = next_arg
                && arg.to_str() == Some("help")
            {
                self.help(&cmds, pipe)?;
                return Ok(None);
            } else {
                0
            }
        } else if let Some(ref arg) = next_arg {
            if let Some(arg) = arg.to_str() {
                if arg.chars().next() != Some('-') {
                    if arg == "help" {
                        if let Some(arg) = args.next()
                            && let Some(arg) = arg.to_str()
                            && let Ok(i) = cmds.binary_search_by(|probe| probe.0.cmp(&Some(arg)))
                            && let Some((Some(_), cmd)) = cmds.get(i)
                        {
                            cmd.help(pipe)?;
                        } else {
                            self.help(&cmds, pipe)?;
                        }
                        return Ok(None);
                    }

                    let cmd = match cmds.binary_search_by(|probe| probe.0.cmp(&Some(arg))) {
                        Ok(i) => {
                            next_arg = args.next();
                            if next_arg.is_none() && cmds[i].1.num_params() > 0 {
                                self.help(&cmds, pipe)?;
                                return Ok(None);
                            }
                            i
                        }
                        Err(i) => {
                            writeln!(pipe, "unknown subcommand \"{arg}\"")?;

                            let mut first = true;
                            let mut iter = cmds[i..].iter();
                            while let Some(cmd) = iter.next()
                                && let Some(cmd) = cmd.0
                                && cmd.starts_with(arg)
                            {
                                if first {
                                    first = false;
                                    writeln!(pipe, "did you mean one of the following?")?;
                                }
                                writeln!(pipe, "    {cmd}")?;
                            }

                            return Err(io::Error::new(io::ErrorKind::InvalidInput, "unknown subcommand"));
                        }
                    };

                    cmd
                } else {
                    // expected subcommand but found switch
                    0
                }
            } else {
                write!(pipe, "invalid utf8 when subcommand or switch was expected")?;
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid utf8"));
            }
        } else {
            self.help(&cmds, pipe)?;
            return Ok(None);
        };

        let mut iter = cmds.into_iter();
        let (_name, cmd) = iter.nth(cmd_index).unwrap();

        let mut i = 0;
        let switches = cmd.sorted_switches;
        for (name, _, _, _) in switches.iter() {
            if name.is_none() {
                break;
            }
            i += 1;
        }
        let mut cmd = Command::<
                MAX_PARAMETERS, MAX_UNIQUE_SWITCHES, MAX_SWITCHES, MAX_SWITCH_PARAMETERS
            >::new(cmd, &switches[..i]);

        let mut current_switch = None;
        while let Some(arg) = next_arg {
            let kind = osstr_switch_kind(&arg);
            if let Some(kind) = kind {
                // TODO: look into better way of partial parsing to allow for different
                // switch syntax mixed with invalid utf8 characters (for example "-d=(unix path)")
                current_switch = if let Some(switch) = arg.to_str() {
                    match kind {
                        SwitchLength::Short => {
                            let switch = &switch[1..2];
                            if switch.len() == 1 {
                                match cmd.switches.binary_search_by(|probe| probe.0.cmp(&Some(switch))) {
                                    Ok(i) => Some(i),
                                    Err(_) => {
                                        writeln!(pipe, "unknown switch \"-{switch}\"")?;
                                        None
                                    }
                                }
                            } else {
                                None
                            }
                        }
                        SwitchLength::Long => {
                            if switch.len() >= 4 {
                                let switch = &switch[2..];
                                match cmd.switches.binary_search_by(|probe| probe.0.cmp(&Some(switch))) {
                                    Ok(i) => Some(i),
                                    Err(_) => {
                                        writeln!(pipe, "unknown switch \"--{switch}\"")?;
                                        None
                                    }
                                }
                            } else {
                                None
                            }
                        }
                    }
                } else {
                    write!(pipe, "switch has invalid utf8 \"{}\"", Path::new(&arg).display())?;
                    None
                };

                if let Some(i) = current_switch {
                    cmd.switch_activate(i);
                }
            } else if let Some(i) = current_switch
                && cmd.switch_params_left(i) > Some(0)
            {
                cmd.insert_switch_param(i, arg);
            } else if cmd.params_left() > 0 {
                cmd.insert_param(arg);
            } else {
                cmd.unused += 1;
                writeln!(pipe, "unused argument \"{}\"", Path::new(&arg).display())?;
            }
            next_arg = args.next();
        }

        pipe.flush()?;
        Ok(Some(cmd))
    }
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn const_switch_sort() {
        const SORTED: [(Option<&'static str>, usize, usize); 7] = insertion_sort_switch([
            (None, 0, 0),
            (Some("d"), 0, 0),
            (Some("e"), 0, 0),
            (None, 0, 0),
            (Some("b"), 0, 0),
            (Some("aa"), 0, 0),
            (Some("a"), 0, 0),
        ]);

        assert_eq!(
            vec![Some("a"), Some("aa"), Some("b"), Some("d"), Some("e"), None, None],
            SORTED.iter().map(|(s, _, _)| *s).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn const_command_builder_sort() {
        const TEST: CommandBuilder = command![Switch::new("test", 0),];
        const SORTED: SortedCommandBuilder<3> = insertion_sort_command_builder([
            (Some("b"), &TEST),
            (Some("a"), &TEST),
            (None, &TEST),
        ]);

        assert_eq!(
            SORTED.0.iter().map(|(s, _)| *s).collect::<Vec<_>>(),
            vec![None, Some("a"), Some("b")],
        );
    }

    fn app() {
        //const BASE: CommandBuilder = command![
        //    Switch::new("new", 1),
        //    Switch::new("new2", 2),
        //    Switch::new("new3", 3),
        //    Switch::new("test_new3", 9),
        //    Switch::new("test_new3", 10),
        //    Switch::new("test_new3", 11),
        //    Switch::new("test_new3", 12),
        //].with_desc("test");
        //
        //const SUB: CommandBuilder = command![
        //    Switch::new("test_new", 2),
        //    Switch::new("test_new2", 6),
        //    Switch::new("test_new3", 8),
        //    Switch::new("test_new3", 9),
        //    Switch::new("test_new3", 10),
        //    Switch::new("test_new3", 11),
        //    Switch::new("test_new3", 12),
        //].with_desc("test");
        //
        //const UNPACK: CommandBuilder = command![
        //    Switch::new("test_new", 2),
        //    Switch::new("test_new2", 6),
        //    Switch::new("test_new3", 8),
        //    Switch::new("test_new3", 9),
        //    Switch::new("test_new3", 10),
        //    Switch::new("test_new3", 11),
        //    Switch::new("test_new3", 12),
        //].with_desc("test");
        //
        //if let Ok(Some(app)) = app![
        //    None = BASE,
        //    Some("sub") = SUB,
        //    Some("unpack") = UNPACK,
        //    Some("u") = UNPACK,
        //] {
        //    println!("{}", app.num_switches());
        //    println!("{}", app.num_parameters());
        //    println!("{}", std::mem::size_of_val(&app));
        //}
    }
}