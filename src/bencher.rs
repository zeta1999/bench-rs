use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use crate::{Stats, Step};
use crate::fmt_thousands_sep;
use crate::timing_future::TimingFuture;
#[cfg(feature = "track-allocator")]
use crate::track_allocator::GLOBAL;

pub struct Bencher {
    pub name: String,
    pub count: usize,
    pub steps: Vec<Step>,
    pub bytes: usize,
    pub n: usize,
    pub poll: usize,
    pub format_fn: fn(&Stats, &Bencher),

    pub mem_track: (&'static AtomicUsize, &'static AtomicUsize)
}

impl Bencher {
    #[cfg(feature = "track-allocator")]
    pub fn new(name: impl AsRef<str>, count: usize, bytes: usize) -> Self {
        Bencher {
            name: name.as_ref().to_owned(),
            count,
            steps: Vec::with_capacity(count),
            bytes,
            n: 0,
            poll: 0,
            format_fn: |s, b| Self::default_format(s, b),

            mem_track: (GLOBAL.counter(), GLOBAL.peak())
        }
    }

    #[cfg(not(feature = "track-allocator"))]
    pub fn new(name: impl AsRef<str>, count: usize, bytes: usize, counter: &'static AtomicUsize, peak: &'static AtomicUsize) -> Self {
        Bencher {
            name: name.as_ref().to_owned(),
            count,
            steps: Vec::with_capacity(count),
            bytes,
            n: 0,
            poll: 0,
            format_fn: |s, b| Self::default_format(s, b),

            mem_track: (counter, peak)
        }
    }

    // (time, memory_usage)
    pub fn bench_once<T>(&self, f: &mut impl FnMut() -> T, n: usize) -> (u128, usize) {
        let now = Instant::now();
        self.reset_mem();

        for _ in 0..n {
            let _output = f();
        }

        (now.elapsed().as_nanos(), self.get_mem_peak())
    }

    pub fn iter<T>(&mut self, mut f: impl FnMut() -> T) {
        let single = self.bench_once(&mut f, 1).0;
        // 1_000_000ns : 1ms
        self.n = (1_000_000 / single.max(1)).max(1) as usize;
        (0..self.count).for_each(|_| {
            let res = self.bench_once(&mut f, self.n);
            self.steps.push(Step {
                time: res.0 / self.n as u128,
                mem: res.1 / self.n
            })
        });
    }

    pub fn async_iter<'a, T, Fut: Future<Output=T>>(&'a mut self, mut f: impl FnMut() -> Fut + 'a) -> impl Future + 'a {
        async move {
            let single = TimingFuture::new(f()).await.elapsed_time.as_nanos();
            // 1_000_000ns : 1ms
            self.n = (1_000_000 / single.max(1)).max(1) as usize;

            let mut polls = Vec::with_capacity(self.count);

            for _ in 0..self.count {
                let mut mtime = 0u128;
                self.reset_mem();
                
                for _ in 0..self.n {
                    let tf = TimingFuture::new(f()).await;
                    mtime += tf.elapsed_time.as_nanos();
                    polls.push(tf.poll);
                }

                self.steps.push(Step {
                    time: mtime / self.n as u128,
                    mem: self.get_mem_peak() / self.n
                });
            }

            self.poll = polls.iter().sum::<usize>() / polls.len();
        }
    }

    pub fn finish(&self) {
        let stats = Stats::from(&self.steps);
        (self.format_fn)(&stats, self)
    }

    pub fn reset_mem(&self) {
        self.mem_track.0.store(0, Ordering::SeqCst);
        self.mem_track.1.store(0, Ordering::SeqCst);
    }

    pub fn get_mem_peak(&self) -> usize {
        self.mem_track.1.load(Ordering::SeqCst)
    }

    fn default_format(stats: &Stats, bencher: &Bencher) {
        bunt::println!(
            "{[bg:white+blue+bold]} ... {[green+underline]} ns/iter (+/- {[red+underline]}) = {[yellow+underline]:.2} MB/s\
            \n\t memory usage: {[green+underline]} bytes/iter (+/- {[red+underline]})\
            \n\t @Total: {[magenta]} * {[white]} iters\
            {[bold]}",
             &bencher.name,
             fmt_thousands_sep(stats.times_average, ','),
             fmt_thousands_sep(stats.times_max - stats.times_min, ','),
             (bencher.bytes as f64 * (1_000_000_000f64 / stats.times_average as f64)) / 1000f64 / 1000f64,

             fmt_thousands_sep(stats.mem_average, ','),
             fmt_thousands_sep(stats.mem_max - stats.mem_min, ','),

             bencher.count,
             bencher.n,

             if bencher.poll > 0 {
                format!(
                    "\n\t @avg {} polls ",
                    bencher.poll
                 )
             } else {
                String::new()
             },
        );
    }
}
