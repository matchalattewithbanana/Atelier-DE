//! Backend-independent, deterministic grid geometry.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Divide the output into rows of balanced tiles. Return None if one-pixel
/// tiles cannot fit, allowing the compositor to retain floating geometry.
pub fn grid(width: i32, height: i32, count: usize) -> Option<Vec<Tile>> {
    if count == 0 { return Some(Vec::new()); }
    if width <= 0 || height <= 0 { return None; }
    let mut columns = 1usize;
    while columns < count.div_ceil(columns) { columns += 1; }
    columns = columns.min(width as usize);
    let rows = count.div_ceil(columns);
    if rows > height as usize { return None; }
    let mut result = Vec::with_capacity(count);
    for row in 0..rows {
        let items = columns.min(count - row * columns);
        let top = (i64::from(height) * row as i64 / rows as i64) as i32;
        let bottom = (i64::from(height) * (row + 1) as i64 / rows as i64) as i32;
        for column in 0..items {
            let left = (i64::from(width) * column as i64 / items as i64) as i32;
            let right = (i64::from(width) * (column + 1) as i64 / items as i64) as i32;
            result.push(Tile { x: left, y: top, width: right - left, height: bottom - top });
        }
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fills_odd_dimensions_without_overlap() {
        for count in 1..20 {
            let tiles = grid(101, 79, count).unwrap();
            assert_eq!(tiles.len(), count);
            assert_eq!(tiles.iter().map(|t| t.width * t.height).sum::<i32>(), 101 * 79);
            for (i, a) in tiles.iter().enumerate() {
                assert!(a.width > 0 && a.height > 0 && a.x + a.width <= 101 && a.y + a.height <= 79);
                for b in &tiles[i+1..] {
                    assert!(a.x + a.width <= b.x || b.x + b.width <= a.x || a.y + a.height <= b.y || b.y + b.height <= a.y);
                }
            }
        }
    }
    #[test]
    fn handles_empty_and_tiny_outputs() {
        assert_eq!(grid(0, 0, 0), Some(vec![]));
        assert!(grid(1, 1, 2).is_none());
        assert!(grid(0, 10, 1).is_none());
    }
}
