use std::{vec::Vec, io::Write};
use std::string::String;
use std::fs::File;
use std::io::LineWriter;
use clap::{Parser, ArgAction};
use serde::{Deserialize};
use rusqlite::{Connection};
use regex::Regex;

/// Program to load data from specified file into accounting database
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args
{
    /// Items database file to load
    #[clap(short, long, parse(from_os_str))]
    database: std::path::PathBuf,

    /// Items price file to load
    #[clap(short, long, parse(from_os_str))]
    filename: std::path::PathBuf,

    /// Items price file to load
    #[clap(short, long, value_parser)]
    user: String,

    /// Whether to only validate the file
    #[clap(short, long, value_parser, action = ArgAction::SetTrue)]
    validate: bool,

    /// Output the table as a markdown table to specified file
    #[clap(short, long, parse(from_os_str))]
    markdown: Option<std::path::PathBuf>,
}

#[derive(Deserialize, Debug)]
struct Item
{
    name: String,
    price: String,
}

struct UserData
{
    user: String,
    total: u32,
    avg_contribution: u32,
    balances: Vec<i32>,
}

fn load_csv(filename: &std::path::PathBuf) -> Vec<Item>
{
    let price_re = Regex::new(r"^\d+.\d{2}$").unwrap();
    let mut items: Vec<Item> = std::vec![];
    let mut rdr = match csv::ReaderBuilder::new()
        .has_headers(false)
        .from_path(filename)
        {
            Ok(r) => r,
            Err(e) => panic!("Could not open CSV file `{:?}`: {}", filename.as_os_str(), e)
        };

    let mut row_idx = 0;
    for result in rdr.deserialize() {
        // Notice that we need to provide a type hint for automatic
        // deserialization.
        let item: Item = match result
        {
            Ok(r) => r,
            Err(e) => panic!("Could not deserialize CSV row with index {}: {}", row_idx, e)
        };
        let price_trimmed = item.price.trim();
        price_to_fixed_point(price_trimmed);
        if !price_re.is_match(price_trimmed)
        {
            panic!("Invalid price for item `{}`. Must be in form `\\d+.\\d{{2}}`", item.name);
        }
        items.push(item);
        row_idx += 1;
    }

    items
}

fn fixed_point_to_price(mut num: i32) -> String
{
    let is_negative = num < 0;
    num = num.abs();

    let mut price = format!("{}", num);
    while price.len() < 3
    {
        price.insert(0, '0');
    }

    price.insert(price.len() - 2, '.');
    if is_negative
    {
        price.insert(0, '-');
    }
    price
}

fn price_to_fixed_point(price_str: &str) -> u32
{
    let fixed_point_str = price_str.replace(".", "");
    match fixed_point_str.parse()
    {
        Ok(i) => i,
        Err(e)     => panic!("Could not parse price `{}`: {}", price_str, e)
    }
}

fn add_items(db: &mut Connection, items: &Vec<Item>, user: &String)
{
    for item in items
    {
        db.execute(
            "
            INSERT INTO account (user, item_name, cost) VALUES (?1, ?2, ?3)
            ",
            &[user, item.name.trim(), price_to_fixed_point(&item.price).to_string().as_str()]
        ).unwrap();
    }
}

// output table to README
fn print_table(db: &mut Connection, path: &std::path::PathBuf)
{
    let file = File::create(path).unwrap();
    let mut writer = LineWriter::new(file);
    let mut stmt = db.prepare("SELECT * FROM account")
        .unwrap();
    writer.write("|id|timestamp|user|item name|cost|\n".as_bytes()).unwrap();
    writer.write("|---|---|---|---|---|\n".as_bytes()).unwrap();
    let mut rows = stmt.query([]).unwrap();

    while let Some(row) = rows.next().unwrap()
    {
        writer.write(format!("|{}|{}|{}|{}|{}|\n",
                             row.get::<_, i64>(0).unwrap(),
                             row.get::<_, String>(1).unwrap(),
                             row.get::<_, String>(2).unwrap(),
                             row.get::<_, String>(3).unwrap(),
                             fixed_point_to_price(row.get::<_, i64>(4).unwrap() as i32))
                     .as_bytes())
            .unwrap();
    }

    writer.flush().unwrap();
}

fn print_csv(user_datas: &Vec<UserData>)
{
    let mut writer = std::io::stdout();

    writer.write(format!("User, Total Contribution, {}\n", user_datas.iter()
            .map(|user_data| user_data.user.clone())
            .collect::<Vec<String>>()
            .join(", ")
        ).as_bytes()
    ).unwrap();

    for user_data in user_datas.iter()
    {
        writer.write(format!("{},{},{}\n",
                             user_data.user,
                             fixed_point_to_price(user_data.total as i32),
                             user_data.balances.iter()
                                .map(|&balance| fixed_point_to_price(balance))
                                .collect::<Vec<String>>()
                                .join(","))
                     .as_bytes())
            .unwrap();

    }
}

fn compute_balances(db: &mut Connection) -> Vec<UserData>
{
    // user, total, balance
    let mut stmt = db.prepare("SELECT COUNT(DISTINCT user) FROM account")
        .unwrap();
    let mut rows = stmt.query([]).unwrap();

    let mut num_users: u32 = 0;
    while let Some(row) = rows.next().unwrap()
    {
        num_users = row.get::<_, i64>(0).unwrap() as u32;
    }

    if num_users == 0
    {
        return vec![];
    }

    let mut stmt = db.prepare("SELECT user, SUM(cost) FROM account GROUP BY user")
        .unwrap();
    let mut rows = stmt.query([]).unwrap();

    let mut user_datas: Vec<UserData> = std::vec![];
    let mut total_contributions = 0;
    while let Some(row) = rows.next().unwrap()
    {
        let user_total = row.get::<_, i64>(1).unwrap() as u32;
        user_datas.push(UserData
            {
                user: row.get::<_, String>(0).unwrap(),
                total: user_total,
                avg_contribution: user_total / num_users,
                balances: vec![],
            }
        );
        total_contributions += user_total as i32;
    }

    let mut new_total_contributions = 0;
    for user_idx in 0..num_users as usize
    {
        let user_total = user_datas[user_idx].total;
        let user_avg_contribution = user_datas[user_idx].avg_contribution;
        user_datas[user_idx].balances = user_datas.iter()
            .map(|debtor| {
                let user_contribution = debtor.avg_contribution as i32 - user_avg_contribution as i32;
                new_total_contributions += user_contribution;
                user_contribution
            })
            .collect::<Vec<i32>>();
        new_total_contributions += user_total as i32;
    }

    // correct for rounding error
    if new_total_contributions != total_contributions
    {
        let diff = new_total_contributions - total_contributions;
        user_datas[num_users as usize - 1].balances[num_users as usize - 1] += diff;
    }

    user_datas
}

fn main()
{
    let args = Args::parse();
    let items = load_csv(&args.filename);
    if args.validate
    {
        std::process::exit(0)
    }

    let db_filename = args.database.clone().into_os_string();
    let mut db = match Connection::open(args.database)
    {
        Ok(c) => c,
        Err(e) => panic!("Could not open database `{:?}`: {}", db_filename, e)
    };
    add_items(&mut db, &items, &args.user);

    if let Some(path) = args.markdown
    {
        print_table(&mut db, &path);
    }

    let balances = compute_balances(&mut db);
    print_csv(&balances);
}
