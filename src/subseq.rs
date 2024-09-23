pub trait SubSequence<T> {
  fn find_slice(&self, s: &[T]) -> Option<usize>;
}

impl<T> SubSequence<T> for [T] 
where 
  T: PartialEq,
{
  fn find_slice(&self, s: &[T]) -> Option<usize> {
    let len = s.len();
    if len == 0 {
      return Some(0);
    }
    if len > self.len() {
      return None;
    }

    let mut iter = self.iter().enumerate();
    let mut offset = 0usize;

    while let Some((i, v)) = iter.next() {
      if *v == s[offset] {
        offset += 1;
        if offset == len {
          return Some(i - len + 1);
        }
      } else {
        offset = 0;
      }
    }
    None
  }
}