/// A sum type that can either be a `Left` or a `Right`
pub enum Either<L, R> {
    Left(L),
    Right(R),
}

impl<L, R> Either<L, R> {
    pub fn unwrap_left(self) -> L {
        match self {
            Either::Left(l) => l,
            Either::Right(_) => panic!("called `Either::unwrap_left()` on a `Right` value"),
        }
    }
}
