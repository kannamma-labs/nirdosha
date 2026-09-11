fn average_score(total_points: i64, num_students: i64) -> i64 {
    total_points / num_students
}

fn main() {
    println!("{}", average_score(275, 4));
}