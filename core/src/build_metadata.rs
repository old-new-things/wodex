//! Build identity validation shared by the host build script and no_std updater.

pub(crate) fn valid_commit(commit: &str) -> bool {
  commit.len() == 40
    && commit.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn valid_timestamp(timestamp: &str) -> bool {
  let bytes = timestamp.as_bytes();
  if bytes.len() != 20
    || bytes[4] != b'-'
    || bytes[7] != b'-'
    || bytes[10] != b'T'
    || bytes[13] != b':'
    || bytes[16] != b':'
    || bytes[19] != b'Z'
    || bytes
      .iter()
      .enumerate()
      .any(|(index, byte)| ![4, 7, 10, 13, 16, 19].contains(&index) && !byte.is_ascii_digit())
  {
    return false;
  }
  let number = |range: core::ops::Range<usize>| {
    bytes[range].iter().fold(0_u32, |value, byte| value * 10 + u32::from(byte - b'0'))
  };
  let year = number(0..4);
  let month = number(5..7);
  let day = number(8..10);
  let days = match month {
    1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
    4 | 6 | 9 | 11 => 30,
    2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
    2 => 28,
    _ => return false,
  };
  year >= 1970
    && day >= 1
    && day <= days
    && number(11..13) < 24
    && number(14..16) < 60
    && number(17..19) < 60
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn timestamps_are_canonical_calendar_times() {
    for timestamp in ["1970-01-01T00:00:00Z", "2000-02-29T23:59:59Z", "2028-02-29T00:00:00Z"] {
      assert!(valid_timestamp(timestamp), "{timestamp}");
    }
    for timestamp in [
      "1969-12-31T23:59:59Z",
      "2026-02-29T00:00:00Z",
      "2100-02-29T00:00:00Z",
      "2026-04-31T00:00:00Z",
      "2026-00-01T00:00:00Z",
      "2026-13-01T00:00:00Z",
      "2026-01-00T00:00:00Z",
      "2026-01-01T24:00:00Z",
      "2026-01-01T00:60:00Z",
      "2026-01-01T00:00:60Z",
      "2026-1-01T00:00:00Z",
      "2026-01-01t00:00:00Z",
      "2026-01-01T00:00:00+00:00",
      "2026-01-01T00:00:00.000Z",
      "用户",
    ] {
      assert!(!valid_timestamp(timestamp), "{timestamp}");
    }
    assert!(valid_commit(&"a0".repeat(20)));
    for commit in ["a".repeat(39), "A".repeat(40), "g".repeat(40), "é".repeat(20)] {
      assert!(!valid_commit(&commit));
    }
  }
}
