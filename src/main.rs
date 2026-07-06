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
#[command(name = "acorns", version, about = "Manage your Acorns account from the terminal")]
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
    /// Print the GraphQL request that would be sent; do not execute
    #[arg(long, global = true)]
    dry_run: bool,
    /// Verbose: log operation name, variables, timing
    #[arg(long, short = 'v', global = true)]
    verbose: bool,
}

impl GlobalOpts {
    fn ctx(&self) -> exec::Ctx {
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
struct AuthCmd { #[command(subcommand)] sub: AuthSub }
#[derive(Subcommand)]
enum AuthSub {
    /// Log in (email + password, then SMS/email MFA)   [write]
    Login {
        #[arg(long)] email: Option<String>,
        /// MFA method to prefer
        #[arg(long, value_enum, default_value_t = MfaMethod::Sms)] method: MfaMethod,
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
enum MfaMethod { Sms, Email }

// ---------- account ----------
#[derive(Args)]
struct AccountCmd { #[command(subcommand)] sub: AccountSub }
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
struct InvestCmd { #[command(subcommand)] sub: InvestSub }
#[derive(Subcommand)]
enum InvestSub {
    /// Invest account balance   [read]
    Balance,
    /// Detailed invest account: portfolio, risk, theme, allocations, projection   [read]
    Account,
    /// Performance over N days   [read]
    Performance { #[arg(long, default_value_t = 30)] days: i64 },
    /// Portfolio fund/holdings breakdown   [read]
    Holdings,
    /// Transaction history   [read]
    History { #[arg(long, default_value_t = 25)] limit: i64, #[arg(long)] after: Option<String>, #[arg(long)] exclude_fees: bool },
    /// Deposit into Invest ("Transfer in")   [MONEY]
    Deposit {
        /// Dollar amount to invest
        amount: f64,
        /// Invest account id (defaults to your primary account)
        #[arg(long)] id: Option<String>,
    },
    /// Withdraw from Invest to a funding source ("Transfer out")   [MONEY]
    Withdraw {
        /// Dollar amount to withdraw ($5 minimum)
        amount: f64,
        /// Destination funding-source UUID (default: your preferred/primary account; see `acorns transfer funding-source`)
        #[arg(long)] to: Option<String>,
        /// Invest account id (defaults to your primary account)
        #[arg(long)] id: Option<String>,
    },
    /// Cancel a pending investment   [write]
    Cancel { investment_id: String },
    /// Recurring investment settings   [read/write]
    Recurring { #[command(subcommand)] sub: RecurringSub },
    /// Portfolio management   [read/write]
    Portfolio { #[command(subcommand)] sub: PortfolioSub },
    /// Round-Ups   [read/write]
    Roundups { #[command(subcommand)] sub: RoundupsSub },
}
#[derive(Copy, Clone, ValueEnum)]
enum Frequency { Daily, Weekly, Monthly }
impl Frequency {
    fn as_api(self) -> &'static str {
        match self {
            Frequency::Daily => "daily",
            Frequency::Weekly => "weekly",
            Frequency::Monthly => "monthly",
        }
    }
}

/// Parse a recurring-investment day: `last` -> -1 (last day of month), else an integer.
fn parse_day(s: &str) -> Result<i64, String> {
    if s.eq_ignore_ascii_case("last") {
        return Ok(-1);
    }
    s.parse::<i64>()
        .map_err(|_| format!("day must be 1–28 or \"last\" (got '{s}')"))
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
#[derive(Subcommand)]
enum RecurringSub {
    /// Current recurring investment settings   [read]
    Status,
    /// Set recurring investment   [MONEY]
    Set {
        /// Dollar amount per investment (e.g. 5 or 5.00)
        #[arg(long)] amount: f64,
        /// Day of month: 1–28, or "last" for the last day (monthly). Day of week for weekly; ignored for daily
        #[arg(long, value_parser = parse_day)] day: i64,
        /// How often to invest
        #[arg(long, value_enum)] frequency: Frequency,
        /// Invest account id (defaults to your primary account)
        #[arg(long)] id: Option<String>,
    },
    /// Stop recurring investment   [write]
    Stop { #[arg(long)] id: Option<String> },
}
#[derive(Subcommand)]
enum PortfolioSub {
    /// Available/selected portfolios   [read]
    Status,
    /// Set portfolio by id   [write]
    Set { id: String },
    /// Update theme/risk   [write]
    Update { #[arg(long)] theme: String, #[arg(long)] risk: String, #[arg(long)] id: Option<String> },
}
#[derive(Subcommand)]
enum RoundupsSub {
    /// Round-Ups settings overview   [read]
    Status,
    /// "Set to automatic" toggle   [write]
    Automatic { #[arg(value_enum)] state: OnOff },
    /// Multiplier: off, 2, 3, or 10   [write]
    Multiplier { #[arg(value_parser = parse_multiplier)] value: i64 },
    /// Whole-Dollar Round-Ups amount ($0.00–$1.00, e.g. 0.50; 0 = off)   [write]
    WholeDollar { amount: f64 },
    /// Linked round-up accounts   [read/write]
    Accounts { #[command(subcommand)] sub: RoundupAccountsSub },
    /// Round-up history   [read]
    History { #[arg(long)] after: Option<String> },
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
enum OnOff { On, Off }

// ---------- funding ----------
#[derive(Args)]
struct FundingCmd { #[command(subcommand)] sub: FundingSub }
#[derive(Subcommand)]
enum FundingSub {
    /// Funding source(s) & linked bank(s)   [read]
    Status,
    /// Set the primary funding source (sub-account id from `funding status`)   [write]
    SetPrimary { id: String },
    /// Allow or pause transfers ("Allow transfers" toggle; global)   [write]
    Allow { #[arg(value_enum)] state: OnOff },
    /// Unlink a bank connection (linkedAccountId from `funding status`; re-link in the app)   [destructive]
    Unlink { linked_account_id: String },
}

// ---------- tax ----------
#[derive(Args)]
struct TaxCmd { #[command(subcommand)] sub: TaxSub }
#[derive(Subcommand)]
enum TaxSub {
    /// Tax statements for a year   [read]
    Statements { #[arg(long)] year: i64, #[arg(long, default_value_t = 25)] limit: i64 },
    /// Download a tax form PDF for a year (fetches a fresh link, saves to disk)   [read]
    Download {
        #[arg(long)] year: i64,
        /// Statement id (required only if a year has multiple forms)
        #[arg(long)] id: Option<String>,
        /// Output path (default: <year>-<formType>-<id>.pdf)
        #[arg(long)] out: Option<String>,
    },
}

// ---------- raw ----------
#[derive(Args)]
struct RawCmd {
    /// Operation name (from the catalog). Omit with --list to browse.
    operation: Option<String>,
    /// Set a variable: --var name=value (repeatable)
    #[arg(long = "var", value_name = "K=V")]
    vars: Vec<String>,
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


