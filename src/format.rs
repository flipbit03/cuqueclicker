const SUFFIXES: &[&str] = &[
    "", "k", "M", "B", "T", "Qa", "Qi", "Sx", "Sp", "Oc", "No", "Dc",
];

pub fn big(n: f64) -> String {
    if n.is_nan() || n.is_infinite() {
        return "?".into();
    }
    if n < 1000.0 {
        return format!("{}", n.floor() as u64);
    }
    let mag = (n.log10() / 3.0).floor() as i32;
    let idx = mag.clamp(0, (SUFFIXES.len() - 1) as i32) as usize;
    let scaled = n / 10f64.powi((idx * 3) as i32);
    format!("{:.2}{}", scaled, SUFFIXES[idx])
}

pub fn rate(n: f64) -> String {
    if n < 10.0 {
        format!("{:.2}", n)
    } else if n < 100.0 {
        format!("{:.1}", n)
    } else {
        big(n)
    }
}

pub fn duration(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{}h {:02}m {:02}s", h, m, s)
    } else if m > 0 {
        format!("{}m {:02}s", m, s)
    } else {
        format!("{}s", s)
    }
}
