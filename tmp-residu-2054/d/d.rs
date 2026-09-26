fn prod_d() {}
#[cfg(all(test, feature = "x"))]
mod tests {
    fn t() {
        debug!("un log dans du code de test, sous une forme cfg que le regex ne matche pas");
    }
}
