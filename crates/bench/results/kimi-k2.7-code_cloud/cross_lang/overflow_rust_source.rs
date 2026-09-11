fn order_total_cents(unit_price_cents: i64, quantity: i64) -> i64 {
    unit_price_cents * quantity
}

fn main() {
    println!("{}", order_total_cents(123456789012345i64, 987654321i64));
}