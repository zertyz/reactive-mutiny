pub mod non_blocking_atomic_stack;
pub mod blocking_stack;
pub mod non_blocking_parking_lot_stack;


/// trait for multi-producer / multi-consumer blocking & non-blocking stacks
pub trait OgreStack<SlotType> {
    fn new(stack_name: String) -> Self where Self: Sized;
    fn push(&self, element: SlotType) -> bool;
    fn pop(&self) -> Option<SlotType>;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool;
    fn buffer_size(&self) -> usize;
    fn debug_enabled(&self) -> bool;
    fn metrics_enabled(&self) -> bool;
    fn stack_name(&self) -> &str;
    fn implementation_name(&self) -> &str;
}

/// trait for multi-producer / multi-consumer blocking stacks
pub trait OgreBlockingStack<SlotType> {
    fn new(stack_name: String) -> Self where Self: Sized;
    fn push(&self, element: SlotType);
    fn pop(&self) -> SlotType;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool;
    fn buffer_size(&self) -> usize;
    fn debug_enabled(&self) -> bool;
    fn metrics_enabled(&self) -> bool;
    fn stack_name(&self) -> &str;
    fn implementation_name(&self) -> &str;
}

// NOTE: we used to have a nice benchmarking code & tests here when there was several non-blocking stack implementations. History can bring it back if needed.