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
#[cfg(test)]
pub fn grid(width:i32,height:i32,count:usize)->Option<Vec<Tile>>{
    spaced_grid(width,height,count,atelier_config::WindowSpacing::default())
}
/// Reduce gaps uniformly as necessary to fit at least one logical pixel per tile.
/// Empty and impossible layouts have the same semantics as the original grid.
pub fn spaced_grid(width: i32, height: i32, count: usize, spacing:atelier_config::WindowSpacing) -> Option<Vec<Tile>> {
    if count == 0 { return Some(Vec::new()); }
    if width <= 0 || height <= 0 { return None; }
    let mut columns = 1usize;
    while columns < count.div_ceil(columns) { columns += 1; }
    columns = columns.min(width as usize);
    let rows = count.div_ceil(columns);
    if rows > height as usize { return None; }
    let columns_i=columns as i32;let rows_i=rows as i32;
    let outer=(spacing.outer_gap.min(64) as i32).min((width-columns_i)/2).min((height-rows_i)/2);
    let w=width-2*outer;let h=height-2*outer;
    let mut inner=spacing.inner_gap.min(48) as i32;
    if columns_i>1{inner=inner.min((w-columns_i)/(columns_i-1));}
    if rows_i>1{inner=inner.min((h-rows_i)/(rows_i-1));}
    let available_h=h-inner*(rows_i-1);
    let mut result = Vec::with_capacity(count);
    for row in 0..rows {
        let items = columns.min(count - row * columns);
        let top = (i64::from(available_h) * row as i64 / rows as i64) as i32;
        let bottom = (i64::from(available_h) * (row + 1) as i64 / rows as i64) as i32;
        let available_w=w-inner*(items as i32-1);
        for column in 0..items {
            let left = (i64::from(available_w) * column as i64 / items as i64) as i32;
            let right = (i64::from(available_w) * (column + 1) as i64 / items as i64) as i32;
            result.push(Tile { x: outer+left+inner*column as i32, y: outer+top+inner*row as i32, width: right - left, height: bottom - top });
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
    fn gaps_are_exact_on_large_outputs_and_reduce_on_tiny_ones(){
        let spacing=atelier_config::WindowSpacing{inner_gap:10,outer_gap:20};
        assert_eq!(spaced_grid(100,100,2,spacing).unwrap(),vec![Tile{x:20,y:20,width:25,height:60},Tile{x:55,y:20,width:25,height:60}]);
        for count in 1..=24{
            let tiles=spaced_grid(7,5,count,spacing).unwrap();
            for (i,a) in tiles.iter().enumerate(){
                assert!(a.x>=0&&a.y>=0&&a.width>0&&a.height>0&&a.x+a.width<=7&&a.y+a.height<=5);
                for b in &tiles[i+1..]{assert!(a.x+a.width<=b.x||b.x+b.width<=a.x||a.y+a.height<=b.y||b.y+b.height<=a.y);}
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
