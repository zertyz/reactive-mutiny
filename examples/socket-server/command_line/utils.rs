//! Contains functions useful for command line parsing and elsewhere --
//! -- like in [crate::config::config_model] structs shared with this module.

use std::{
    ops::{RangeInclusive},
    ffi::OsStr,
    str::FromStr,
};
use std::str::Split;
use std::time::Duration;
use chrono::{NaiveDate};


/// parses `arg` in the form "2022-03-16..=2022-03-31"
pub fn range_inclusive_date_from_str(arg: &OsStr) -> RangeInclusive<NaiveDate> {
    let mut split = arg.to_str().unwrap().split("..=");
    let mut parse_next_date = || match split.next() {
        Some(str_date) => NaiveDate::parse_from_str(str_date, "%Y-%m-%d")
            .map_err(|err| format!("Could not parse date '{}' from RangeInclusive dates '{:?}': {}", str_date, arg, err))
            .unwrap(),
        None => panic!("{:?} is not a RangeInclusive<NaiveDate>. Inclusive Ranges are in the form \"start..=finish\"", arg)
    };
    let first_date = parse_next_date();
    let last_date = parse_next_date();
    first_date ..= last_date
}

/// parses `arg` in the form "2022-05-29" and returns a [NaiveDate]
pub fn naive_date_from_str(arg: &OsStr) -> NaiveDate {
    let str_date = arg.to_str().unwrap();
    NaiveDate::parse_from_str(str_date, "%Y-%m-%d")
        .map_err(|err| format!("Could not parse date '{}': {}", str_date, err))
        .unwrap()
}

/// parses `arg` in the form "01:02:03.004005" and returns a [Duration]
pub fn duration_from_str(arg: &OsStr) -> Duration {
    let mut split = arg.to_str().unwrap().split(":");
    let parse_next_number = |split: &mut Split<&str>| {
        match split.next() {
            Some(string_number) => string_number.parse::<f64>()
                .map_err(|err| format!("Could not parse number '{}' from Duration '{:?}'", string_number, err))
                .unwrap(),
            None => panic!("{} is not a Duration -- which should be in the form \"HH:MM:SS.µs\" -- like \"01:02:03.004005\"", arg.to_str().unwrap())
        }
    };
    let h = parse_next_number(&mut split);
    let m = parse_next_number(&mut split);
    let s = parse_next_number(&mut split);
    Duration::from_secs_f64((h * 3600.0) + (m * 60.0) + s)
}

/// parses `arg` in the form `X1..=Xn` (where `X` is a number) and returns the corresponding `Range<NumberType>` object
pub fn range_inclusive_number_from_str<NumberType:FromStr>(arg: &OsStr) -> RangeInclusive<NumberType> where <NumberType as FromStr>::Err: std::fmt::Debug {
    let mut split = arg.to_str().unwrap().split("..=");  // '.'
    let mut parse_next_number = || match split.next() {
        Some(string_number) => string_number.parse::<NumberType>()
            .map_err(|err| format!("Could not parse number '{}' in RangeInclusive '{:?}'", string_number, err))
            .unwrap(),
        None => panic!("{} is not a RangeInclusive<Number>. Inclusive Ranges are in the form \"start..=finish\"", arg.to_str().unwrap())
    };
    let start = parse_next_number();
    let finish = parse_next_number();
    start ..= finish
}

/// parses `arg` in the form `X1,X2,X3, ..., Xn` (where `X` is a number) and returns the corresponding `Vec<NumberType>` object
pub fn list_from_str<NumberType:FromStr>(arg: &OsStr) -> Vec<NumberType> where <NumberType as FromStr>::Err: std::fmt::Debug {
    let split = arg.to_str().unwrap().split(",");
    split
        .map(|string_number| string_number.parse::<NumberType>().unwrap())
        .collect()
}

