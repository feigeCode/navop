---
name: gpui-performance
description: Performance optimization techniques for GPUI including rendering optimization, layout performance, memory management, and profiling strategies. Use when user needs to optimize GPUI application performance or debug performance issues.
---

# GPUI Performance Optimization

## Metadata

This skill provides comprehensive guidance on optimizing GPUI applications for rendering performance, memory efficiency, and overall runtime speed.

## Instructions

### Rendering Optimization

#### Understanding the Render Cycle

```
State Change → cx.notify() → Render → Layout → Paint → Display
```

**Key Points**:
- Only call `cx.notify()` when state actually changes
- Minimize work in `render()` method
- Cache expensive computations
- Reduce element count and nesting

#### Avoiding Unnecessary Renders

```rust
// BAD: Renders on every frame
impl MyComponent {
    fn start_animation(&mut self, cx: &mut ViewContext<Self>) {
        cx.spawn(|this, mut cx| async move {
            loop {
                cx.update(|_, cx| cx.notify()).ok();  // Forces rerender!
                Timer::after(Duration::from_millis(16)).await;
            }
        }).detach();
    }
}

// GOOD: Only render when state changes
impl MyComponent {
    fn update_value(&mut self, new_value: i32, cx: &mut ViewContext<Self>) {
        if self.value != new_value {
            self.value = new_value;
            cx.notify();  // Only notify on actual change
        }
    }
}
```

#### Optimize Subscription Updates

```rust
// BAD: Always rerenders on model change
let _subscription = cx.observe(&model, |_, _, cx| {
    cx.notify();  // Rerenders even if nothing relevant changed
});

// GOOD: Selective updates
let _subscription = cx.observe(&model, |this, model, cx| {
    let data = model.read(cx);

    // Only rerender if relevant field changed
    if data.relevant_field != this.cached_field {
        this.cached_field = data.relevant_field.clone();
        cx.notify();
    }
});
```

#### Memoization Pattern

```rust
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

struct MemoizedComponent {
    model: Model<Data>,
    cached_result: RefCell<Option<(u64, String)>>,  // (hash, result)
}

impl MemoizedComponent {
    fn expensive_computation(&self, cx: &ViewContext<Self>) -> String {
        let data = self.model.read(cx);

        // Calculate hash of input
        let mut hasher = DefaultHasher::new();
        data.relevant_fields.hash(&mut hasher);
        let hash = hasher.finish();

        // Return cached if unchanged
        if let Some((cached_hash, cached_result)) = &*self.cached_result.borrow() {
            if *cached_hash == hash {
                return cached_result.clone();
            }
        }

        // Compute and cache
        let result = perform_expensive_computation(&data);
        *self.cached_result.borrow_mut() = Some((hash, result.clone()));
        result
    }
}
```

### Layout Performance

#### Minimize Layout Complexity

```rust
// BAD: Deep nesting
div()
    .flex()
    .child(
        div()
            .flex()
            .child(
                div()
                    .flex()
                    .child(
                        div().child("Content")
                    )
            )
    )

// GOOD: Flat structure
div()
    .flex()
    .flex_col()
    .gap_4()
    .child("Header")
    .child("Content")
    .child("Footer")
```

#### Use Fixed Sizing When Possible

```rust
// BETTER: Fixed sizes (no layout calculation)
div()
    .w(px(200.))
    .h(px(100.))
    .child("Fixed size")

// SLOWER: Dynamic sizing (requires layout calculation)
div()
    .w_full()
    .h_full()
    .child("Dynamic size")
```

#### Avoid Layout Thrashing

```rust
// BAD: Reading layout during render
impl Render for BadComponent {
    fn render(&mut self, cx: &mut ViewContext<Self>) -> impl IntoElement {
        let width = cx.window_bounds().get_bounds().size.width;
        // Using width immediately causes layout thrashing
        div().w(width)
    }
}

// GOOD: Cache layout-dependent values
struct GoodComponent {
    cached_width: Pixels,
}

impl GoodComponent {
    fn on_window_resize(&mut self, cx: &mut ViewContext<Self>) {
        let width = cx.window_bounds().get_bounds().size.width;
        if self.cached_width != width {
            self.cached_width = width;
            cx.notify();
        }
    }
}
```

#### Virtual Scrolling for Long Lists

```rust
struct VirtualList {
    items: Vec<String>,
    scroll_offset: f32,
    viewport_height: f32,
    item_height: f32,
}

impl Render for VirtualList {
    fn render(&mut self, cx: &mut ViewContext<Self>) -> impl IntoElement {
        // Calculate visible range
        let start_index = (self.scroll_offset / self.item_height).floor() as usize;
        let visible_count = (self.viewport_height / self.item_height).ceil() as usize;
        let end_index = (start_index + visible_count).min(self.items.len());

        // Only render visible items
        div()
            .h(px(self.viewport_height))
            .overflow_y_scroll()
            .on_scroll(cx.listener(|this, event, cx| {
                this.scroll_offset = event.scroll_offset.y;
                cx.notify();
            }))
            .child(
                div()
                    .h(px(self.items.len() as f32 * self.item_height))
                    .child(
                        div()
                            .absolute()
                            .top(px(start_index as f32 * self.item_height))
                            .children(
                                self.items[start_index..end_index]
                                    .iter()
                                    .map(|item| {
                                        div()
                                            .h(px(self.item_height))
                                            .child(item.as_str())
                                    })
                            )
                    )
            )
    }
}
```

### Memory Management

#### Preventing Memory Leaks

```rust
// LEAK: Subscription not stored
impl BadView {
    fn new(model: Model<Data>, cx: &mut ViewContext<Self>) -> Self {
        cx.observe(&model, |_, _, cx| cx.notify());  // Leak!
        Self { model }
    }
}

// CORRECT: Store subscription
struct GoodView {
    model: Model<Data>,
    _subscription: Subscription,  // Cleaned up on Drop
}

impl GoodView {
    fn new(model: Model<Data>, cx: &mut ViewContext<Self>) -> Self {
        let _subscription = cx.observe(&model, |_, _, cx| cx.notify());
        Self { model, _subscription }
    }
}
```

#### Avoid Circular References

```rust
// BAD: Circular reference
struct CircularRef {
    self_view: Option<View<Self>>,  // Circular!
}

// GOOD: Use weak references or redesign
struct NoCycle {
    other_view: View<OtherView>,  // No cycle
}
```

#### Bounded Collections

```rust
use std::collections::VecDeque;

const MAX_HISTORY: usize = 100;

struct BoundedHistory {
    items: VecDeque<Item>,
}

impl BoundedHistory {
    fn add_item(&mut self, item: Item) {
        self.items.push_back(item);

        // Maintain size limit
        while self.items.len() > MAX_HISTORY {
            self.items.pop_front();
        }
    }
}
```

#### Reuse Allocations

```rust
struct BufferedComponent {
    buffer: String,  // Reused across operations
}

impl BufferedComponent {
    fn format_data(&mut self, data: &[Item]) -> &str {
        self.buffer.clear();  // Reuse allocation

        for item in data {
            use std::fmt::Write;
            write!(&mut self.buffer, "{}\n", item.name).ok();
        }

        &self.buffer
    }
}
```

### Profiling Strategies

#### CPU Profiling with cargo-flamegraph

```bash
# Install
cargo install flamegraph

# Profile application
cargo flamegraph --bin your-app

# With specific features
cargo flamegraph --bin your-app --features profiling

# Opens flamegraph.svg showing CPU time distribution
```

#### Memory Profiling

```bash
# valgrind (Linux)
valgrind --tool=massif --massif-out-file=massif.out ./target/release/your-app
ms_print massif.out

# heaptrack (Linux)
heaptrack ./target/release/your-app
heaptrack_gui heaptrack.your-app.*.gz

# Instruments (macOS)
instruments -t "Allocations" ./target/release/your-app
```

#### Custom Performance Monitoring

```rust
use std::time::Instant;

struct PerformanceMonitor {
    frame_times: VecDeque<Duration>,
    max_samples: usize,
}

impl PerformanceMonitor {
    fn new() -> Self {
        Self {
            frame_times: VecDeque::with_capacity(100),
            max_samples: 100,
        }
    }

    fn record_frame(&mut self, duration: Duration) {
        self.frame_times.push_back(duration);

        if self.frame_times.len() > self.max_samples {
            self.frame_times.pop_front();
        }

        // Warn if frame is slow (> 16ms for 60fps)
        if duration.as_millis() > 16 {
            eprintln!("⚠️  Slow frame: {}ms", duration.as_millis());
        }
    }

    fn average_fps(&self) -> f64 {
        if self.frame_times.is_empty() {
            return 0.0;
        }

        let total: Duration = self.frame_times.iter().sum();
        let avg = total / self.frame_times.len() as u32;
        1000.0 / avg.as_millis() as f64
    }

    fn percentile(&self, p: f64) -> Duration {
        let mut sorted: Vec<_> = self.frame_times.iter().copied().collect();
        sorted.sort();

        let index = (sorted.len() as f64 * p) as usize;
        sorted[index.min(sorted.len() - 1)]
    }
}

// Usage in component
impl MyView {
    fn measure_render<F>(&mut self, f: F, cx: &mut ViewContext<Self>)
    where
        F: FnOnce(&mut Self, &mut ViewContext<Self>)
    {
        let start = Instant::now();
        f(self, cx);
        let elapsed = start.elapsed();

        self.perf_monitor.record_frame(elapsed);

        // Log stats periodically
        if self.frame_count % 60 == 0 {
            println!(
                "Avg FPS: {:.1}, p95: {}ms, p99: {}ms",
                self.perf_monitor.average_fps(),
                self.perf_monitor.percentile(0.95).as_millis(),
                self.perf_monitor.percentile(0.99).as_millis(),
            );
        }
    }
}
```

#### Benchmark with Criterion

```rust
// benches/component_bench.rs
use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};

fn render_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("rendering");

    for size in [10, 100, 1000].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(size),
            size,
            |b, &size| {
                b.iter(|| {
                    App::test(|cx| {
                        let items = vec![Item::default(); size];
                        let view = cx.new_view(|cx| {
                            ListView::new(items, cx)
                        });

                        view.update(cx, |view, cx| {
                            black_box(view.render(cx));
                        });
                    });
                });
            }
        );
    }

    group.finish();
}

criterion_group!(benches, render_benchmark);
criterion_main!(benches);
```

### Batching Updates

```rust
// BAD: Multiple individual updates
for item in items {
    self.model.update(cx, |model, cx| {
        model.add_item(item);  // Triggers rerender each time!
        cx.notify();
    });
}

// GOOD: Batch into single update
self.model.update(cx, |model, cx| {
    for item in items {
        model.add_item(item);
    }
    cx.notify();  // Single rerender
});
```

### Async Rendering Optimization

```rust
struct AsyncView {
    loading_state: Model<LoadingState>,
}

impl AsyncView {
    fn load_data(&mut self, cx: &mut ViewContext<Self>) {
        let loading_state = self.loading_state.clone();

        // Show loading immediately
        self.loading_state.update(cx, |state, cx| {
            *state = LoadingState::Loading;
            cx.notify();
        });

        // Load asynchronously
        cx.spawn(|_, mut cx| async move {
            // Fetch data
            let data = fetch_data().await?;

            // Update state once
            cx.update_model(&loading_state, |state, cx| {
                *state = LoadingState::Loaded(data);
                cx.notify();
            })?;

            Ok::<_, anyhow::Error>(())
        }).detach();
    }
}
```

### Caching Strategies

#### Result Caching

```rust
use std::collections::HashMap;

struct CachedRenderer {
    cache: RefCell<HashMap<String, CachedElement>>,
}

impl CachedRenderer {
    fn render_cached(
        &self,
        key: String,
        render_fn: impl FnOnce() -> AnyElement,
    ) -> AnyElement {
        let mut cache = self.cache.borrow_mut();

        cache.entry(key)
            .or_insert_with(|| CachedElement::new(render_fn()))
            .element
            .clone()
    }

    fn invalidate(&self, key: &str) {
        self.cache.borrow_mut().remove(key);
    }
}
```

## Resources

### Performance Targets

**Rendering**:
- Target: 60 FPS (16.67ms per frame)
- Render + Layout: ~10ms
- Paint: ~6ms
- Warning: Any frame > 16ms

**Memory**:
- Monitor heap growth
- Warning: Steady increase (leak)
- Target: Stable after initialization

**Startup**:
- Window display: < 100ms
- Fully interactive: < 500ms

### Profiling Tools

**CPU Profiling**:
- cargo-flamegraph: Visualize CPU time
- perf (Linux): System-level profiling
- Instruments (macOS): Apple's profiler

**Memory Profiling**:
- valgrind/massif: Memory usage tracking
- heaptrack: Heap allocation tracking
- Instruments: Memory allocations

**Benchmarking**:
- criterion: Statistical benchmarking
- cargo bench: Built-in benchmarks
- hyperfine: Command-line tool benchmarking

### Best Practices

1. **Measure First**: Profile before optimizing
2. **Minimize Renders**: Only `cx.notify()` when necessary
3. **Cache Results**: Memoize expensive computations
4. **Batch Updates**: Group state changes
5. **Virtual Scrolling**: For long lists
6. **Flat Layouts**: Avoid deep nesting
7. **Fixed Sizing**: When possible
8. **Monitor Memory**: Watch for leaks
9. **Async Loading**: Don't block UI
10. **Test Performance**: Include benchmarks

### Common Bottlenecks

- Subscription in render (memory leak)
- Expensive computation in render
- Deep component nesting
- Unnecessary rerenders
- Layout thrashing
- Large lists without virtualization
- Memory leaks from circular refs
- Unbounded collections