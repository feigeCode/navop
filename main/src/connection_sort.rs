use std::cmp::Ordering;
use std::iter::Peekable;
use std::str::Chars;

/// 连接名称排序比较：数字段按数值比较（对 IP 地址友好，如 "172.29.13.2" < "172.29.13.100"），
/// 其余部分按字符比较（忽略大小写）。
pub(crate) fn connection_name_cmp(left: &str, right: &str) -> Ordering {
    natural_cmp(left, right)
}

fn natural_cmp(left: &str, right: &str) -> Ordering {
    let mut left_chars = left.chars().peekable();
    let mut right_chars = right.chars().peekable();
    loop {
        match (left_chars.peek().copied(), right_chars.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(left), Some(right)) if left.is_ascii_digit() && right.is_ascii_digit() => {
                match compare_number_runs(&mut left_chars, &mut right_chars) {
                    Ordering::Equal => {}
                    other => return other,
                }
            }
            (Some(left), Some(right)) => {
                match left.to_ascii_lowercase().cmp(&right.to_ascii_lowercase()) {
                    Ordering::Equal => {}
                    other => return other,
                }
                left_chars.next();
                right_chars.next();
            }
        }
    }
}

/// 比较两个连续数字段（如 IP 段 "2" 与 "100"），按数值大小比较。
fn compare_number_runs(
    left: &mut Peekable<Chars<'_>>,
    right: &mut Peekable<Chars<'_>>,
) -> Ordering {
    let left_run = take_number_run(left);
    let right_run = take_number_run(right);
    left_run
        .len
        .cmp(&right_run.len)
        .then(left_run.value.cmp(&right_run.value))
}

fn take_number_run(chars: &mut Peekable<Chars<'_>>) -> NumberRun {
    let mut len = 0usize;
    let mut value = 0u128;
    while let Some(&digit) = chars.peek() {
        if !digit.is_ascii_digit() {
            break;
        }
        len += 1;
        value = value
            .saturating_mul(10)
            .saturating_add(u128::from(digit.to_digit(10).unwrap_or(0)));
        chars.next();
    }
    NumberRun { len, value }
}

struct NumberRun {
    len: usize,
    value: u128,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(mut names: Vec<&str>) -> Vec<&str> {
        names.sort_by(|left, right| connection_name_cmp(left, right));
        names
    }

    #[test]
    fn ip_addresses_sort_numerically_by_segment() {
        let names = sorted(vec![
            "172.29.13.100",
            "172.29.13.2",
            "172.29.9.1",
            "172.29.13.200",
        ]);

        assert_eq!(
            vec![
                "172.29.9.1",
                "172.29.13.2",
                "172.29.13.100",
                "172.29.13.200"
            ],
            names,
        );
    }

    #[test]
    fn hostnames_with_numbers_sort_naturally() {
        let names = sorted(vec!["host-10", "host-2", "host-1"]);

        assert_eq!(vec!["host-1", "host-2", "host-10"], names);
    }

    #[test]
    fn plain_names_sort_lexically_case_insensitive() {
        let names = sorted(vec!["Redis", "db-server", "App Server", "Db-Server"]);

        assert_eq!(vec!["App Server", "db-server", "Db-Server", "Redis"], names);
    }

    #[test]
    fn identical_names_are_equal() {
        assert_eq!(Ordering::Equal, connection_name_cmp("db-1", "db-1"));
    }
}
