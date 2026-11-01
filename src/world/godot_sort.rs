//! Godot's Array.sort_custom (core/templates/sort_array.h): an introsort that is NOT stable
//! for arrays longer than 16. The generator's sorts can tie, so matching the original's
//! layouts needs the same algorithm.

pub fn sort_custom<T: Clone>(v: &mut [T], less: impl Fn(&T, &T) -> bool) {
    let n = v.len() as i64;
    if n == 0 {
        return;
    }
    let s = Sorter { less: &less };
    s.introsort(0, n, v, bitlog(n) * 2);
    s.final_insertion_sort(0, n, v);
}

fn bitlog(mut n: i64) -> i64 {
    let mut k = 0;
    while n != 1 {
        n >>= 1;
        k += 1;
    }
    k
}

const THRESHOLD: i64 = 16;

struct Sorter<'a, T> {
    less: &'a dyn Fn(&T, &T) -> bool,
}

impl<T: Clone> Sorter<'_, T> {
    fn c(&self, a: &T, b: &T) -> bool {
        (self.less)(a, b)
    }

    fn median_of_3(&self, p: &[T], a: i64, b: i64, c: i64) -> i64 {
        let (x, y, z) = (&p[a as usize], &p[b as usize], &p[c as usize]);
        if self.c(x, y) {
            if self.c(y, z) {
                b
            } else if self.c(x, z) {
                c
            } else {
                a
            }
        } else if self.c(x, z) {
            a
        } else if self.c(y, z) {
            c
        } else {
            b
        }
    }

    fn push_heap(&self, first: i64, mut hole: i64, top: i64, value: T, a: &mut [T]) {
        let mut parent = (hole - 1) / 2;
        while hole > top && self.c(&a[(first + parent) as usize], &value) {
            a[(first + hole) as usize] = a[(first + parent) as usize].clone();
            hole = parent;
            parent = (hole - 1) / 2;
        }
        a[(first + hole) as usize] = value;
    }

    fn adjust_heap(&self, first: i64, mut hole: i64, len: i64, value: T, a: &mut [T]) {
        let top = hole;
        let mut second = 2 * hole + 2;
        while second < len {
            if self.c(
                &a[(first + second) as usize],
                &a[(first + second - 1) as usize],
            ) {
                second -= 1;
            }
            a[(first + hole) as usize] = a[(first + second) as usize].clone();
            hole = second;
            second = 2 * (second + 1);
        }
        if second == len {
            a[(first + hole) as usize] = a[(first + second - 1) as usize].clone();
            hole = second - 1;
        }
        self.push_heap(first, hole, top, value, a);
    }

    fn pop_heap_to(&self, first: i64, last: i64, result: i64, value: T, a: &mut [T]) {
        a[result as usize] = a[first as usize].clone();
        self.adjust_heap(first, 0, last - first, value, a);
    }

    fn pop_heap(&self, first: i64, last: i64, a: &mut [T]) {
        let v = a[(last - 1) as usize].clone();
        self.pop_heap_to(first, last - 1, last - 1, v, a);
    }

    fn make_heap(&self, first: i64, last: i64, a: &mut [T]) {
        if last - first < 2 {
            return;
        }
        let len = last - first;
        let mut parent = (len - 2) / 2;
        loop {
            let v = a[(first + parent) as usize].clone();
            self.adjust_heap(first, parent, len, v, a);
            if parent == 0 {
                return;
            }
            parent -= 1;
        }
    }

    fn partial_sort(&self, first: i64, last: i64, middle: i64, a: &mut [T]) {
        self.make_heap(first, middle, a);
        for i in middle..last {
            if self.c(&a[i as usize], &a[first as usize]) {
                let v = a[i as usize].clone();
                self.pop_heap_to(first, middle, i, v, a);
            }
        }
        let mut l = middle;
        while l - first > 1 {
            self.pop_heap(first, l, a);
            l -= 1;
        }
    }

    fn partitioner(&self, mut first: i64, mut last: i64, pivot: i64, a: &mut [T]) -> i64 {
        let mut pivot_loc = pivot;
        loop {
            while first != pivot && self.c(&a[first as usize], &a[pivot_loc as usize]) {
                first += 1;
            }
            last -= 1;
            while last != pivot && self.c(&a[pivot_loc as usize], &a[last as usize]) {
                last -= 1;
            }
            if first >= last {
                return first;
            }
            if pivot_loc == first {
                pivot_loc = last;
            } else if pivot_loc == last {
                pivot_loc = first;
            }
            a.swap(first as usize, last as usize);
            first += 1;
        }
    }

    fn introsort(&self, first: i64, mut last: i64, a: &mut [T], mut depth: i64) {
        while last - first > THRESHOLD {
            if depth == 0 {
                self.partial_sort(first, last, last, a);
                return;
            }
            depth -= 1;
            let m = self.median_of_3(a, first, first + (last - first) / 2, last - 1);
            let cut = self.partitioner(first, last, m, a);
            self.introsort(cut, last, a, depth);
            last = cut;
        }
    }

    fn unguarded_linear_insert(&self, mut last: i64, value: T, a: &mut [T]) {
        let mut next = last - 1;
        while self.c(&value, &a[next as usize]) {
            a[last as usize] = a[next as usize].clone();
            last = next;
            next -= 1;
        }
        a[last as usize] = value;
    }

    fn linear_insert(&self, first: i64, last: i64, a: &mut [T]) {
        let val = a[last as usize].clone();
        if self.c(&val, &a[first as usize]) {
            let mut i = last;
            while i > first {
                a[i as usize] = a[(i - 1) as usize].clone();
                i -= 1;
            }
            a[first as usize] = val;
        } else {
            self.unguarded_linear_insert(last, val, a);
        }
    }

    fn insertion_sort(&self, first: i64, last: i64, a: &mut [T]) {
        if first == last {
            return;
        }
        let mut i = first + 1;
        while i != last {
            self.linear_insert(first, i, a);
            i += 1;
        }
    }

    fn final_insertion_sort(&self, first: i64, last: i64, a: &mut [T]) {
        if last - first > THRESHOLD {
            self.insertion_sort(first, first + THRESHOLD, a);
            for i in (first + THRESHOLD)..last {
                let v = a[i as usize].clone();
                self.unguarded_linear_insert(i, v, a);
            }
        } else {
            self.insertion_sort(first, last, a);
        }
    }
}
