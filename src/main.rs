//! acorns — unofficial CLI for an Acorns account (GraphQL backend).
//!
//! Command tree skeleton. Handlers are stubbed for now; wired incrementally.

use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};

mod account;
mod auth;
mod catalog;
mod cmd;
mod exec;
mod funding;
mod invest;
mod net;
mod output;
mod raw;
mod safety;
mod session;
mod tax;

#[derive(Parser)]
#[command(
    name = "acorns",
    version,
    about = "Manage your Acorns account from the terminal"
)]
struct Cli {
    #[command(flatten)]
    global: GlobalOpts,
    #[command(subcommand)]
    command: Command,
}

#[derive(Args, Clone)]
struct GlobalOpts {
    /// Skip confirmation prompts (money/destructive ops)
    #[arg(long, short = 'y', global = true)]
    yes: bool,
    /// Print the GraphQL request that would be sent; no network I/O, ever
    #[arg(long, global = true)]
    dry_run: bool,
    /// Verbose: log operation name, variables (secrets redacted), timing
    #[arg(long, short = 'v', global = true)]
    verbose: bool,
}

impl GlobalOpts {
    const fn ctx(&self) -> exec::Ctx {
        exec::Ctx {
            dry_run: self.dry_run,
            verbose: self.verbose,
        }
    }
}

#[derive(Subcommand)]
enum Command {
    /// Authentication & session
    Auth(AuthCmd),
    /// Account overview
    Account(AccountCmd),
    /// Investing: balance, performance, deposits, round-ups, portfolio
    Invest(InvestCmd),
    /// Funding source & linked bank(s)
    Funding(FundingCmd),
    /// Tax documents & statements
    Tax(TaxCmd),
    /// Escape hatch: run any cataloged GraphQL operation by name
    Raw(RawCmd),
    /// Generate a shell completion script (e.g. `acorns completions bash`)
    Completions {
        /// Shell to generate completions for
        shell: clap_complete::Shell,
    },
}

// ---------- auth ----------
#[derive(Args)]
struct AuthCmd {
    #[command(subcommand)]
    sub: AuthSub,
}

#[derive(Subcommand)]
enum AuthSub {
    /// Log in (email + password, then SMS/email MFA)   [write]
    Login {
        #[arg(long)]
        email: Option<String>,
        /// MFA method to prefer
        #[arg(long, value_enum, default_value_t = MfaMethod::Sms)]
        method: MfaMethod,
    },
    /// Log out and clear the session   [write]
    Logout,
    /// Rotate the access token   [write]
    Refresh,
    /// Show current session status   [read]
    Status,
    /// Change account password   [destructive]
    Password,
}

#[derive(Copy, Clone, ValueEnum)]
enum MfaMethod {
    Sms,
    Email,
}

// ---------- account ----------
#[derive(Args)]
struct AccountCmd {
    #[command(subcommand)]
    sub: AccountSub,
}

#[derive(Subcommand)]
enum AccountSub {
    /// Total Acorns value   [read]
    Value,
    /// List all accounts   [read]
    List,
    /// Next subscription billing date   [read]
    Billing,
}

// ---------- invest ----------
#[derive(Args)]
struct InvestCmd {
    #[command(subcommand)]
    sub: InvestSub,
}

#[derive(Subcommand)]
enum InvestSub {
    /// Invest account balance   [read]
    Balance,
    /// Detailed invest account: portfolio, risk, theme, allocations, projection   [read]
    Account,
    /// Performance over N days   [read]
    Performance {
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(i64).range(1..))]
        days: i64,
    },
    /// Portfolio fund/holdings breakdown   [read]
    Holdings,
    /// Transaction history   [read]
    History {
        #[arg(long, default_value_t = 25, value_parser = clap::value_parser!(i64).range(1..))]
        limit: i64,
        #[arg(long)]
        exclude_fees: bool,
    },
    /// Deposit into Invest ("Transfer in")   [MONEY]
    Deposit {
        /// Dollar amount to invest
        #[arg(value_parser = parse_amount)]
        amount: f64,
        /// Invest account id (defaults to your primary account)
        #[arg(long)]
        id: Option<String>,
    },
    /// Withdraw from Invest to a funding source ("Transfer out")   [MONEY]
    Withdraw {
        /// Dollar amount to withdraw ($5 minimum)
        #[arg(value_parser = parse_withdraw_amount)]
        amount: f64,
        /// Destination funding-source UUID (default: your preferred/primary account; see `acorns funding status`)
        #[arg(long)]
        to: Option<String>,
        /// Invest account id (defaults to your primary account)
        #[arg(long)]
        id: Option<String>,
    },
    /// Cancel a pending investment   [write]
    Cancel { investment_id: String },
    /// Recurring investment settings   [read/write]
    Recurring {
        #[command(subcommand)]
        sub: RecurringSub,
    },
    /// Portfolio management   [read/write]
    Portfolio {
        #[command(subcommand)]
        sub: PortfolioSub,
    },
    /// Round-Ups   [read/write]
    Roundups {
        #[command(subcommand)]
        sub: RoundupsSub,
    },
}

#[derive(Copy, Clone, ValueEnum)]
enum Frequency {
    Daily,
    Weekly,
    Monthly,
}

impl Frequency {
    const fn as_api(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Monthly => "monthly",
        }
    }
}

/// Parse a recurring-investment day: `last` -> -1 (last day of month), else 1–28.
fn parse_day(s: &str) -> Result<i64, String> {
    if s.eq_ignore_ascii_case("last") {
        return Ok(-1);
    }
    match s.parse::<i64>() {
        Ok(d @ 1..=28) => Ok(d),
        _ => Err(format!("day must be 1–28 or \"last\" (got '{s}')")),
    }
}

/// Parse a Round-Ups multiplier: `off` -> 1, else one of the UI options (2, 3, 10).
fn parse_multiplier(s: &str) -> Result<i64, String> {
    match s {
        "off" | "1" => Ok(1),
        "2" => Ok(2),
        "3" => Ok(3),
        "10" => Ok(10),
        _ => Err(format!("multiplier must be off, 2, 3, or 10 (got '{s}')")),
    }
}

/// True when `v` is representable in whole cents (two decimal places).
fn whole_cents(v: f64) -> bool {
    let cents = v * 100.0;
    (cents - cents.round()).abs() < 1e-9
}

/// Parse a positive dollar amount with at most two decimal places. Rejects the
/// values `f64` happily parses but the API cannot mean: NaN, ±inf, 0, negatives.
fn parse_amount(s: &str) -> Result<f64, String> {
    let v: f64 = s.parse().map_err(|_| format!("invalid amount '{s}'"))?;
    if !v.is_finite() || v <= 0.0 {
        return Err(format!(
            "amount must be a positive dollar amount (got '{s}')"
        ));
    }
    if !whole_cents(v) {
        return Err(format!(
            "amount must have at most two decimal places (got '{s}')"
        ));
    }
    Ok(v)
}

/// Like `parse_amount`, plus the documented $5 withdrawal minimum.
fn parse_withdraw_amount(s: &str) -> Result<f64, String> {
    let v = parse_amount(s)?;
    if v < 5.0 {
        return Err(format!("withdrawals have a $5 minimum (got ${v:.2})"));
    }
    Ok(v)
}

/// Whole-Dollar Round-Ups accept $0.00–$1.00 (0 = off).
fn parse_whole_dollar(s: &str) -> Result<f64, String> {
    let v: f64 = s.parse().map_err(|_| format!("invalid amount '{s}'"))?;
    if !v.is_finite() || !(0.0..=1.0).contains(&v) {
        return Err(format!("amount must be between 0.00 and 1.00 (got '{s}')"));
    }
    if !whole_cents(v) {
        return Err(format!(
            "amount must have at most two decimal places (got '{s}')"
        ));
    }
    Ok(v)
}

#[derive(Subcommand)]
enum RecurringSub {
    /// Current recurring investment settings   [read]
    Status,
    /// Set recurring investment   [MONEY]
    Set {
        /// Dollar amount per investment (e.g. 5 or 5.00)
        #[arg(long, value_parser = parse_amount)]
        amount: f64,
        /// Day of month: 1–28, or "last" for the last day (monthly). Day of week for weekly; ignored for daily
        #[arg(long, value_parser = parse_day)]
        day: i64,
        /// How often to invest
        #[arg(long, value_enum)]
        frequency: Frequency,
        /// Invest account id (defaults to your primary account)
        #[arg(long)]
        id: Option<String>,
    },
    /// Stop recurring investment   [write]
    Stop,
}

#[derive(Subcommand)]
enum PortfolioSub {
    /// Available/selected portfolios   [read]
    Status,
    /// Set portfolio by id   [write]
    Set { id: String },
    /// Update theme/risk   [write]
    Update {
        /// Portfolio theme, as shown by `acorns invest portfolio status`
        #[arg(long)]
        theme: String,
        /// Risk level, as shown by `acorns invest portfolio status`
        #[arg(long)]
        risk: String,
        #[arg(long)]
        id: Option<String>,
    },
}

#[derive(Subcommand)]
enum RoundupsSub {
    /// Round-Ups settings overview   [read]
    Status,
    /// "Set to automatic" toggle   [write]
    Automatic {
        #[arg(value_enum)]
        state: OnOff,
    },
    /// Multiplier: off, 2, 3, or 10   [write]
    Multiplier {
        #[arg(value_parser = parse_multiplier)]
        value: i64,
    },
    /// Whole-Dollar Round-Ups amount ($0.00–$1.00, e.g. 0.50; 0 = off)   [write]
    WholeDollar {
        #[arg(value_parser = parse_whole_dollar)]
        amount: f64,
    },
    /// Linked round-up accounts   [read/write]
    Accounts {
        #[command(subcommand)]
        sub: RoundupAccountsSub,
    },
    /// Round-up history   [read]
    History,
}

#[derive(Subcommand)]
enum RoundupAccountsSub {
    /// List linked round-up accounts + their enabled state   [read]
    Status,
    /// Enable a linked account for round-ups   [write]
    Enable { id: String },
    /// Disable a linked account for round-ups   [write]
    Disable { id: String },
}

#[derive(Copy, Clone, ValueEnum)]
enum OnOff {
    On,
    Off,
}

// ---------- funding ----------
#[derive(Args)]
struct FundingCmd {
    #[command(subcommand)]
    sub: FundingSub,
}

#[derive(Subcommand)]
enum FundingSub {
    /// Funding source(s) & linked bank(s)   [read]
    Status,
    /// Set the primary funding source (sub-account id from `funding status`)   [write]
    SetPrimary { id: String },
    /// Allow or pause transfers ("Allow transfers" toggle; global)   [write]
    Allow {
        #[arg(value_enum)]
        state: OnOff,
    },
    /// Unlink a bank connection (linkedAccountId from `funding status`; re-link in the app)   [destructive]
    Unlink { linked_account_id: String },
}

// ---------- tax ----------
#[derive(Args)]
struct TaxCmd {
    #[command(subcommand)]
    sub: TaxSub,
}

#[derive(Subcommand)]
enum TaxSub {
    /// Tax statements for a year   [read]
    Statements {
        /// Tax year (e.g. 2023)
        #[arg(value_parser = clap::value_parser!(i64).range(2000..=2100))]
        year: i64,
    },
    /// Download a tax form PDF for a year (fetches a fresh link, saves to disk)   [read]
    Download {
        /// Tax year (e.g. 2023)
        #[arg(value_parser = clap::value_parser!(i64).range(2000..=2100))]
        year: i64,
        /// Statement id (required only if a year has multiple forms)
        #[arg(long)]
        id: Option<String>,
        /// Output path (default: <year>-<formType>-<id>.pdf)
        #[arg(long)]
        out: Option<String>,
    },
}

// ---------- raw ----------
#[derive(Args)]
struct RawCmd {
    /// Operation name (from the catalog). Omit with --list to browse.
    operation: Option<String>,
    /// Set a variable, parsed as JSON when possible: --var name=value (repeatable)
    #[arg(long = "var", value_name = "K=V")]
    vars: Vec<String>,
    /// Set a variable as a literal string, no JSON parsing: --var-str note=true (repeatable)
    #[arg(long = "var-str", value_name = "K=V")]
    vars_str: Vec<String>,
    /// Full variables object as JSON
    #[arg(long)]
    vars_json: Option<String>,
    /// List cataloged operations (optionally filter by --domain)
    #[arg(long)]
    list: bool,
    /// Filter --list by domain
    #[arg(long)]
    domain: Option<String>,
    /// Describe an operation (kind, variables, input shapes, doc)
    #[arg(long)]
    describe: bool,
}

fn main() {
    let cli = Cli::parse();
    let g = &cli.global;
    if g.verbose {
        eprintln!("(verbose) dry_run={}", g.dry_run);
    }
    let result: anyhow::Result<()> = match &cli.command {
        Command::Raw(c) => raw::run(g, c),
        Command::Auth(c) => auth::run(g, c),
        Command::Account(c) => account::run(g, c),
        Command::Invest(c) => invest::run(g, c),
        Command::Funding(c) => funding::run(g, c),
        Command::Tax(c) => tax::run(g, c),
        Command::Completions { shell } => {
            let mut cmd = Cli::command();
            let bin = cmd.get_name().to_string();
            clap_complete::generate(*shell, &mut cmd, bin, &mut std::io::stdout());
            Ok(())
        }
    };
    if let Err(e) = result {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse_amount, parse_day, parse_multiplier, parse_whole_dollar, parse_withdraw_amount,
    };

    #[test]
    fn day_accepts_documented_range() {
        assert_eq!(parse_day("1"), Ok(1));
        assert_eq!(parse_day("28"), Ok(28));
        assert_eq!(parse_day("last"), Ok(-1));
        assert_eq!(parse_day("LAST"), Ok(-1));
    }

    #[test]
    fn day_rejects_out_of_range() {
        assert!(parse_day("0").is_err());
        assert!(parse_day("29").is_err());
        assert!(parse_day("31").is_err());
        assert!(parse_day("-3").is_err());
        assert!(parse_day("first").is_err());
    }

    #[test]
    fn multiplier_options() {
        assert_eq!(parse_multiplier("off"), Ok(1));
        assert_eq!(parse_multiplier("1"), Ok(1));
        assert_eq!(parse_multiplier("2"), Ok(2));
        assert_eq!(parse_multiplier("3"), Ok(3));
        assert_eq!(parse_multiplier("10"), Ok(10));
        assert!(parse_multiplier("4").is_err());
        assert!(parse_multiplier("").is_err());
    }

    #[test]
    fn amount_accepts_dollars_and_cents() {
        assert_eq!(parse_amount("5"), Ok(5.0));
        assert_eq!(parse_amount("5.00"), Ok(5.0));
        assert_eq!(parse_amount("10.05"), Ok(10.05));
        assert_eq!(parse_amount("0.01"), Ok(0.01));
    }

    #[test]
    fn amount_rejects_nan_inf_zero_negative_subcent() {
        for bad in [
            "NaN", "inf", "-inf", "0", "0.0", "-5", "5.999", "0.001", "abc", "",
        ] {
            assert!(parse_amount(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn withdraw_enforces_minimum() {
        assert_eq!(parse_withdraw_amount("5"), Ok(5.0));
        assert!(parse_withdraw_amount("4.99").is_err());
    }

    #[test]
    fn whole_dollar_range() {
        assert_eq!(parse_whole_dollar("0"), Ok(0.0));
        assert_eq!(parse_whole_dollar("0.50"), Ok(0.5));
        assert_eq!(parse_whole_dollar("1"), Ok(1.0));
        assert!(parse_whole_dollar("1.01").is_err());
        assert!(parse_whole_dollar("-0.5").is_err());
        assert!(parse_whole_dollar("NaN").is_err());
        assert!(parse_whole_dollar("0.505").is_err());
    }
}
