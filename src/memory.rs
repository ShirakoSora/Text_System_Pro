use core::fmt;

pub const PAGE_SHIFT: u32 = 12;
pub const PAGE_SIZE: u64 = 1 << PAGE_SHIFT;
pub const MAX_ORDER: u8 = 32;
pub const MAX_FREE_BLOCKS: usize = 512;
pub const MAX_ALLOCATIONS: usize = 512;
pub const MAX_MEMORY_REGIONS: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameBlock {
    pub start_frame: u64,
    pub order: u8,
}

impl FrameBlock {
    pub const EMPTY: Self = Self {
        start_frame: 0,
        order: 0,
    };

    pub const fn page_count(self) -> u64 {
        1u64 << self.order
    }

    pub const fn start_address(self) -> u64 {
        self.start_frame << PAGE_SHIFT
    }

    pub const fn size_bytes(self) -> u64 {
        self.page_count() << PAGE_SHIFT
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryError {
    InvalidOrder,
    EmptyRegion,
    AddressOverflow,
    OverlappingRegion,
    RegionCapacityExceeded,
    FreeBlockCapacityExceeded,
    AllocationCapacityExceeded,
    OutOfMemory,
    UnknownAllocation,
    FreeBlockOverlap,
}

impl fmt::Display for MemoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidOrder => "invalid buddy allocation order",
            Self::EmptyRegion => "memory region contains no complete pages",
            Self::AddressOverflow => "physical memory region overflows",
            Self::OverlappingRegion => "physical memory regions overlap",
            Self::RegionCapacityExceeded => "physical memory region table is full",
            Self::FreeBlockCapacityExceeded => "buddy free-list capacity is exhausted",
            Self::AllocationCapacityExceeded => "outstanding allocation table is full",
            Self::OutOfMemory => "no matching free physical pages",
            Self::UnknownAllocation => "free does not match an outstanding allocation",
            Self::FreeBlockOverlap => "freed pages overlap a free block",
        })
    }
}

#[derive(Clone, Copy)]
struct MemoryRegion {
    start_frame: u64,
    end_frame: u64,
}

impl MemoryRegion {
    const EMPTY: Self = Self {
        start_frame: 0,
        end_frame: 0,
    };
}

#[derive(Clone)]
pub struct BuddyAllocator {
    free: [FrameBlock; MAX_FREE_BLOCKS],
    free_len: usize,
    allocations: [FrameBlock; MAX_ALLOCATIONS],
    allocation_len: usize,
    regions: [MemoryRegion; MAX_MEMORY_REGIONS],
    region_len: usize,
    total_pages: u64,
    free_pages: u64,
}

impl BuddyAllocator {
    pub const fn new() -> Self {
        Self {
            free: [FrameBlock::EMPTY; MAX_FREE_BLOCKS],
            free_len: 0,
            allocations: [FrameBlock::EMPTY; MAX_ALLOCATIONS],
            allocation_len: 0,
            regions: [MemoryRegion::EMPTY; MAX_MEMORY_REGIONS],
            region_len: 0,
            total_pages: 0,
            free_pages: 0,
        }
    }

    pub const fn total_bytes(&self) -> u64 {
        self.total_pages * PAGE_SIZE
    }

    pub const fn free_bytes(&self) -> u64 {
        self.free_pages * PAGE_SIZE
    }

    pub const fn allocated_bytes(&self) -> u64 {
        (self.total_pages - self.free_pages) * PAGE_SIZE
    }

    pub const fn free_block_count(&self) -> usize {
        self.free_len
    }

    pub fn add_region(&mut self, start_address: u64, size_bytes: u64) -> Result<(), MemoryError> {
        let end_address = start_address
            .checked_add(size_bytes)
            .ok_or(MemoryError::AddressOverflow)?;
        let start_frame = start_address
            .checked_add(PAGE_SIZE - 1)
            .ok_or(MemoryError::AddressOverflow)?
            >> PAGE_SHIFT;
        let end_frame = end_address >> PAGE_SHIFT;

        if start_frame >= end_frame {
            return Err(MemoryError::EmptyRegion);
        }
        if self.region_len == MAX_MEMORY_REGIONS {
            return Err(MemoryError::RegionCapacityExceeded);
        }
        if self.regions[..self.region_len]
            .iter()
            .any(|region| start_frame < region.end_frame && end_frame > region.start_frame)
        {
            return Err(MemoryError::OverlappingRegion);
        }

        let checkpoint = self.clone();
        let pages = end_frame - start_frame;
        self.total_pages = self
            .total_pages
            .checked_add(pages)
            .ok_or(MemoryError::AddressOverflow)?;

        self.regions[self.region_len] = MemoryRegion {
            start_frame,
            end_frame,
        };
        self.region_len += 1;

        let result = self.add_free_range(start_frame, end_frame);
        if result.is_err() {
            *self = checkpoint;
        }
        result
    }

    pub fn allocate(&mut self, order: u8) -> Result<FrameBlock, MemoryError> {
        if order > MAX_ORDER {
            return Err(MemoryError::InvalidOrder);
        }
        if self.allocation_len == MAX_ALLOCATIONS {
            return Err(MemoryError::AllocationCapacityExceeded);
        }

        let source_index = (0..self.free_len)
            .filter(|&index| {
                let block = self.free[index];
                block.order >= order
                    && self.free_len + usize::from(block.order - order) <= MAX_FREE_BLOCKS
            })
            .min_by_key(|&index| self.free[index].order)
            .ok_or(MemoryError::OutOfMemory)?;
        let mut block = self.remove_free(source_index);

        while block.order > order {
            block.order -= 1;
            let buddy = FrameBlock {
                start_frame: block.start_frame + (1u64 << block.order),
                order: block.order,
            };
            self.insert_free(buddy)?;
        }

        self.free_pages -= block.page_count();
        self.allocations[self.allocation_len] = block;
        self.allocation_len += 1;
        Ok(block)
    }

    pub fn free(&mut self, block: FrameBlock) -> Result<(), MemoryError> {
        if block.order > MAX_ORDER {
            return Err(MemoryError::InvalidOrder);
        }
        let allocation_index = self.allocations[..self.allocation_len]
            .iter()
            .position(|&allocated| allocated == block)
            .ok_or(MemoryError::UnknownAllocation)?;

        let allocation_end = block.start_frame + block.page_count();
        if self.free[..self.free_len].iter().any(|free| {
            let free_end = free.start_frame + free.page_count();
            block.start_frame < free_end && allocation_end > free.start_frame
        }) {
            return Err(MemoryError::FreeBlockOverlap);
        }

        self.insert_free(block)?;
        self.remove_allocation(allocation_index);
        self.free_pages += block.page_count();
        Ok(())
    }

    fn add_free_range(&mut self, mut start_frame: u64, end_frame: u64) -> Result<(), MemoryError> {
        self.free_pages += end_frame - start_frame;

        while start_frame < end_frame {
            let remaining_order = (63 - (end_frame - start_frame).leading_zeros()) as u8;
            let aligned_order = if start_frame == 0 {
                MAX_ORDER
            } else {
                start_frame.trailing_zeros().min(u32::from(MAX_ORDER)) as u8
            };
            let order = remaining_order.min(aligned_order).min(MAX_ORDER);
            let block = FrameBlock { start_frame, order };
            if let Err(error) = self.insert_free(block) {
                self.free_pages -= end_frame - start_frame;
                return Err(error);
            }
            start_frame += block.page_count();
        }

        Ok(())
    }

    fn insert_free(&mut self, mut block: FrameBlock) -> Result<(), MemoryError> {
        while block.order < MAX_ORDER {
            let buddy_start = block.start_frame ^ block.page_count();
            let buddy_index = self.free[..self.free_len].iter().position(|candidate| {
                candidate.start_frame == buddy_start && candidate.order == block.order
            });
            let Some(index) = buddy_index else {
                break;
            };

            self.remove_free(index);
            block.start_frame = block.start_frame.min(buddy_start);
            block.order += 1;
        }

        if self.free_len == MAX_FREE_BLOCKS {
            return Err(MemoryError::FreeBlockCapacityExceeded);
        }
        self.free[self.free_len] = block;
        self.free_len += 1;
        Ok(())
    }

    fn remove_free(&mut self, index: usize) -> FrameBlock {
        let removed = self.free[index];
        self.free_len -= 1;
        self.free[index] = self.free[self.free_len];
        self.free[self.free_len] = FrameBlock::EMPTY;
        removed
    }

    fn remove_allocation(&mut self, index: usize) {
        self.allocation_len -= 1;
        self.allocations[index] = self.allocations[self.allocation_len];
        self.allocations[self.allocation_len] = FrameBlock::EMPTY;
    }
}

impl Default for BuddyAllocator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u64 = 0x1000_0000;

    fn allocator(pages: u64) -> BuddyAllocator {
        let mut allocator = BuddyAllocator::new();
        allocator
            .add_region(BASE, pages * PAGE_SIZE)
            .expect("valid test memory region");
        allocator
    }

    #[test]
    fn splits_aligned_blocks_and_coalesces_on_free() {
        let mut allocator = allocator(64);
        assert_eq!(allocator.free_block_count(), 1);

        let block = allocator.allocate(3).unwrap();
        assert_eq!(block.start_address(), BASE);
        assert_eq!(block.size_bytes(), 8 * PAGE_SIZE);
        assert_eq!(allocator.free_bytes(), 56 * PAGE_SIZE);

        allocator.free(block).unwrap();
        assert_eq!(allocator.free_bytes(), 64 * PAGE_SIZE);
        assert_eq!(allocator.free_block_count(), 1);
    }

    #[test]
    fn selects_larger_aligned_region_when_first_region_is_fragmented() {
        let mut allocator = BuddyAllocator::new();
        allocator
            .add_region(BASE + PAGE_SIZE, 15 * PAGE_SIZE)
            .unwrap();
        let block = allocator.allocate(2).unwrap();

        assert_eq!(block.size_bytes(), 4 * PAGE_SIZE);
        assert_eq!(block.start_frame % 4, 0);
        assert_eq!(allocator.free_bytes(), 11 * PAGE_SIZE);
    }

    #[test]
    fn rejects_double_frees_and_unowned_blocks() {
        let mut allocator = allocator(8);
        let block = allocator.allocate(1).unwrap();

        allocator.free(block).unwrap();
        assert_eq!(allocator.free(block), Err(MemoryError::UnknownAllocation));
        assert_eq!(allocator.allocated_bytes(), 0);
    }

    #[test]
    fn rejects_overlapping_memory_map_regions_without_changing_accounting() {
        let mut allocator = allocator(8);

        assert_eq!(
            allocator.add_region(BASE + PAGE_SIZE, 4 * PAGE_SIZE),
            Err(MemoryError::OverlappingRegion)
        );
        assert_eq!(allocator.total_bytes(), 8 * PAGE_SIZE);
        assert_eq!(allocator.free_bytes(), 8 * PAGE_SIZE);
    }

    #[test]
    fn handles_non_power_of_two_memory_regions() {
        let mut allocator = allocator(13);

        assert_eq!(allocator.total_bytes(), 13 * PAGE_SIZE);
        for _ in 0..13 {
            allocator.allocate(0).unwrap();
        }
        assert_eq!(allocator.allocate(0), Err(MemoryError::OutOfMemory));
        assert_eq!(allocator.free_bytes(), 0);
    }

    #[test]
    fn neighbouring_efi_regions_coalesce_into_one_larger_block() {
        let mut allocator = BuddyAllocator::new();
        allocator.add_region(BASE, 8 * PAGE_SIZE).unwrap();
        allocator
            .add_region(BASE + 8 * PAGE_SIZE, 8 * PAGE_SIZE)
            .unwrap();

        assert_eq!(allocator.free_block_count(), 1);
        assert_eq!(
            allocator.allocate(4).unwrap(),
            FrameBlock {
                start_frame: BASE >> PAGE_SHIFT,
                order: 4,
            }
        );
    }

    #[test]
    fn aligns_partial_physical_pages_without_marking_them_allocatable() {
        let mut allocator = BuddyAllocator::new();
        allocator.add_region(BASE + 1, PAGE_SIZE * 3).unwrap();

        assert_eq!(allocator.total_bytes(), 2 * PAGE_SIZE);
        assert_eq!(
            allocator.allocate(0).unwrap().start_address(),
            BASE + PAGE_SIZE
        );
        assert_eq!(
            allocator.allocate(0).unwrap().start_address(),
            BASE + 2 * PAGE_SIZE
        );
    }
}
