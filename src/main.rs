//! acorns — unofficial CLI for an Acorns account (GraphQL backend).
//!
//! Command tree skeleton. Handlers are stubbed for now; wired incrementally.

use clap::{Parser, Subcommand, Args, ValueEnum};

mod account;
mod auth;
mod catalog;
mod cmd;
mod exec;
mod invest;
mod net;
mod output;
mod raw;
mod safety;
mod session;
mod tax;
mod transfer;

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
    /// Transfers & funding sources
    Transfer(TransferCmd),
    /// Tax documents & statements
    Tax(TaxCmd),
    /// Escape hatch: run any cataloged GraphQL operation by name
    Raw(RawCmd),
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
    /// Show current session status   [read]
    Status,
    /// Rotate the access token   [write]
    Refresh,
    /// Log out and clear the session   [write]
    Logout,
    /// Change account password   [destructive]
    Password { #[command(subcommand)] sub: PasswordSub },
}
#[derive(Subcommand)]
enum PasswordSub { Change }
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
    Balance { #[arg(long)] id: Option<String> },
    /// Detailed invest account   [read]
    Account { #[arg(long)] id: Option<String> },
    /// Performance over N days   [read]
    Performance { #[arg(long, default_value_t = 30)] days: i64, #[arg(long)] id: Option<String> },
    /// Portfolio fund/holdings breakdown   [read]
    Holdings,
    /// Transaction history   [read]
    History { #[arg(long, default_value_t = 25)] limit: i64, #[arg(long)] after: Option<String>, #[arg(long)] exclude_fees: bool },
    /// One-time deposit   [MONEY]
    Deposit { amount: f64, #[arg(long)] id: Option<String> },
    /// Withdraw to a funding source   [MONEY]
    Withdraw { amount: f64, #[arg(long)] to: String, #[arg(long)] id: Option<String> },
    /// Cancel a pending investment   [write]
    Cancel { investment_id: String },
    /// Pause/resume recurring deposits   [write]
    Deposits { #[arg(value_enum)] state: PauseState },
    /// Recurring investment settings   [read/write]
    Recurring { #[command(subcommand)] sub: RecurringSub },
    /// Portfolio management   [read/write]
    Portfolio { #[command(subcommand)] sub: PortfolioSub },
    /// Round-Ups   [read/write]
    Roundups { #[command(subcommand)] sub: RoundupsSub },
}
#[derive(Copy, Clone, ValueEnum)]
enum PauseState { Pause, Resume }
#[derive(Subcommand)]
enum RecurringSub {
    /// Show current recurring settings   [read]
    Show,
    /// Set recurring investment   [MONEY]
    Set { #[arg(long)] amount: f64, #[arg(long)] day: i64, #[arg(long)] frequency: String, #[arg(long)] id: Option<String> },
    /// Stop recurring investment   [write]
    Stop { #[arg(long)] id: Option<String> },
}
#[derive(Subcommand)]
enum PortfolioSub {
    /// Show available/selected portfolios   [read]
    Show,
    /// Set portfolio by id   [write]
    Set { id: String },
    /// Update theme/risk   [write]
    Update { #[arg(long)] theme: String, #[arg(long)] risk: String, #[arg(long)] id: Option<String> },
}
#[derive(Subcommand)]
enum RoundupsSub {
    /// Round-up profile status   [read]
    Status,
    /// Enable/disable + multiplier   [write]
    Set { #[arg(long)] enabled: Option<bool>, #[arg(long)] multiplier: Option<i64> },
    /// Whole-dollar round-ups on/off   [write]
    WholeDollar { #[arg(value_enum)] state: OnOff },
    /// Round-up history   [read]
    History { #[arg(long)] after: Option<String> },
}
#[derive(Copy, Clone, ValueEnum)]
enum OnOff { On, Off }

// ---------- transfer ----------
#[derive(Args)]
struct TransferCmd { #[command(subcommand)] sub: TransferSub }
#[derive(Subcommand)]
enum TransferSub {
    /// Create a transfer   [MONEY]
    Create { #[arg(long)] from: String, #[arg(long)] to: String, #[arg(long)] amount: f64 },
    /// List transferable-from (and optionally -to) accounts   [read]
    Accounts { #[arg(long)] to: Option<String> },
    /// Show funding sources   [read]
    FundingSource { #[command(subcommand)] sub: Option<FundingSourceSub> },
    /// Estimate settlement dates   [read]
    Estimate { #[arg(long)] created_at: Option<String> },
    /// Search financial institutions   [read]
    Institutions { #[arg(long)] search: Option<String> },
    /// Recurring transfers   [read/write]
    Recurring { #[command(subcommand)] sub: TransferRecurringSub },
    /// Unlink a linked account   [write]
    Unlink { linked_account_id: String },
}
#[derive(Subcommand)]
enum FundingSourceSub {
    /// Set primary funding source   [write]
    SetPrimary { id: String },
}
#[derive(Subcommand)]
enum TransferRecurringSub {
    /// List recurring transfers   [read]
    List,
    /// Cancel a recurring transfer   [write]
    Cancel { id: String },
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
        Command::Transfer(c) => transfer::run(g, c),
        Command::Tax(c) => tax::run(g, c),
    };
    if let Err(e) = result {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}


