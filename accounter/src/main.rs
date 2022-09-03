use std::{vec::Vec, io::Write};
use std::string::String;
use std::fs::File;
use std::io::LineWriter;
use clap::{Parser, ArgAction};
use serde::{Deserialize, Serialize};
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

#[derive(Serialize, Debug)]
struct UserData
{
    user: String,
    total: u32,
    balance: i32,
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

    for user_data in user_datas.iter()
    {
        writer.write(format!("{},{},{}\n",
                             user_data.user,
                             fixed_point_to_price(user_data.total as i32),
                             fixed_point_to_price(user_data.balance))
                     .as_bytes())
            .unwrap();

    }
}

fn compute_balances(db: &mut Connection) -> Vec<UserData>
{
    // user, total, balance
    let mut user_data: Vec<UserData> = std::vec![];
    let mut sum: u32 = 0;

    let mut stmt = db.prepare("SELECT user, SUM(cost) FROM account GROUP BY user")
        .unwrap();
    let mut rows = stmt.query([]).unwrap();

    while let Some(row) = rows.next().unwrap()
    {
        let user_total = row.get::<_, i64>(1).unwrap() as u32;
        user_data.push(UserData
            {
                user: row.get::<_, String>(0).unwrap(),
                total: user_total,
                balance: 0,
            }
        );
        sum += user_total;
    }

    let avg = sum / user_data.len() as u32;
    for ud in user_data.iter_mut()
    {
        ud.balance = ud.total as i32 - avg as i32;
    }

    user_data
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
