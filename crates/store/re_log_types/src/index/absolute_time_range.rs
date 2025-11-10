use std::ops::RangeInclusive;

use crate::{NonMinI64, TimeInt, TimeReal};

// ----------------------------------------------------------------------------

/// An absolute time range using [`NonMinI64`].
///
/// Can be resolved from [`re_types_core::datatypes::TimeRange`] (which *may* have relative bounds) using a given timeline & cursor.
///
/// This range is guaranteed to never include [`TimeInt::STATIC`] - it represents only temporal values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct AbsoluteTimeRange {
    pub min: NonMinI64,
    pub max: NonMinI64,
}

impl AbsoluteTimeRange {
    /// Contains no time at all.
    pub const EMPTY: Self = Self {
        min: NonMinI64::MAX,
        max: NonMinI64::MIN,
    };

    /// Contains all time.
    pub const EVERYTHING: Self = Self {
        min: NonMinI64::MIN,
        max: NonMinI64::MAX,
    };

    /// Creates a new temporal [`AbsoluteTimeRange`].
    #[inline]
    pub fn new(min: i64, max: i64) -> Self {
        Self {
            min: NonMinI64::saturating_from_i64(min),
            max: NonMinI64::saturating_from_i64(max),
        }
    }

    /// Creates a new [`AbsoluteTimeRange`] from [`NonMinI64`] values.
    #[inline]
    pub fn from_non_min(min: NonMinI64, max: NonMinI64) -> Self {
        Self { min, max }
    }

    /// Creates a point range (min == max).
    #[inline]
    pub fn point(time: i64) -> Self {
        let time = NonMinI64::saturating_from_i64(time);
        Self {
            min: time,
            max: time,
        }
    }

    #[inline]
    pub fn min(&self) -> NonMinI64 {
        self.min
    }

    #[inline]
    pub fn max(&self) -> NonMinI64 {
        self.max
    }

    /// Overwrites the start bound of the range.
    #[inline]
    pub fn set_min(&mut self, time: i64) {
        self.min = NonMinI64::saturating_from_i64(time);
    }

    /// Overwrites the end bound of the range.
    #[inline]
    pub fn set_max(&mut self, time: i64) {
        self.max = NonMinI64::saturating_from_i64(time);
    }

    /// The amount of time or sequences covered by this range.
    #[inline]
    pub fn abs_length(&self) -> u64 {
        self.min.get().abs_diff(self.max.get())
    }

    #[inline]
    pub fn center(&self) -> NonMinI64 {
        self.min.midpoint(self.max)
    }

    #[inline]
    pub fn contains(&self, time: NonMinI64) -> bool {
        self.min <= time && time <= self.max
    }

    /// Does this range fully contain the other?
    #[inline]
    pub fn contains_range(&self, other: Self) -> bool {
        self.min <= other.min && other.max <= self.max
    }

    #[inline]
    pub fn intersects(&self, other: Self) -> bool {
        self.min <= other.max && self.max >= other.min
    }

    #[inline]
    pub fn intersection(&self, other: Self) -> Option<Self> {
        self.intersects(other).then(|| Self {
            min: self.min.max(other.min),
            max: self.max.min(other.max),
        })
    }

    #[inline]
    pub fn union(&self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    pub fn from_relative_time_range(
        range: &re_types_core::datatypes::TimeRange,
        cursor: impl Into<re_types_core::datatypes::TimeInt>,
    ) -> Self {
        let cursor = cursor.into();

        let mut min = range.start.start_boundary_time(cursor);
        let mut max = range.end.end_boundary_time(cursor);

        if min > max {
            std::mem::swap(&mut min, &mut max);
        }

        Self::new(min.0, max.0)
    }
}

impl re_byte_size::SizeBytes for AbsoluteTimeRange {
    #[inline]
    fn heap_size_bytes(&self) -> u64 {
        0
    }
}

// ----------------------------------------------------------------------------

/// Like [`AbsoluteTimeRange`], but using [`TimeReal`] for improved precision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct AbsoluteTimeRangeF {
    pub min: TimeReal,
    pub max: TimeReal,
}

impl AbsoluteTimeRangeF {
    #[inline]
    pub fn new(min: impl Into<TimeReal>, max: impl Into<TimeReal>) -> Self {
        Self {
            min: min.into(),
            max: max.into(),
        }
    }

    #[inline]
    pub fn point(value: impl Into<TimeReal>) -> Self {
        let value = value.into();
        Self {
            min: value,
            max: value,
        }
    }

    /// Inclusive
    pub fn contains(&self, value: TimeReal) -> bool {
        self.min <= value && value <= self.max
    }

    /// Returns the point in the center of the range.
    pub fn center(&self) -> TimeReal {
        self.min.midpoint(self.max)
    }

    /// Where in the range is this value? Returns 0-1 if within the range.
    ///
    /// Returns <0 if before and >1 if after.
    pub fn inverse_lerp(&self, value: TimeReal) -> f64 {
        if self.min == self.max {
            0.5
        } else {
            (value - self.min).as_f64() / (self.max - self.min).as_f64()
        }
    }

    pub fn lerp(&self, t: f64) -> TimeReal {
        self.min + (self.max - self.min) * t
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.min == self.max
    }

    /// The amount of time or sequences covered by this range.
    #[inline]
    pub fn length(&self) -> TimeReal {
        self.max - self.min
    }

    /// Creates an [`AbsoluteTimeRange`] from self by rounding the start
    /// of the range down, and rounding the end of the range up.
    pub fn to_int(self) -> AbsoluteTimeRange {
        AbsoluteTimeRange::new(self.min.floor(), self.max.ceil())
    }
}

impl From<AbsoluteTimeRangeF> for RangeInclusive<TimeReal> {
    fn from(range: AbsoluteTimeRangeF) -> Self {
        range.min..=range.max
    }
}

impl From<&AbsoluteTimeRangeF> for RangeInclusive<TimeReal> {
    fn from(range: &AbsoluteTimeRangeF) -> Self {
        range.min..=range.max
    }
}

impl From<AbsoluteTimeRange> for AbsoluteTimeRangeF {
    fn from(range: AbsoluteTimeRange) -> Self {
        Self::new(range.min.get(), range.max.get())
    }
}
