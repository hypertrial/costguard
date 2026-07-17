use crate::estimate::{sum_lognormals, Estimate};

/// Internal USD/month estimate. Public structs keep their existing `Estimate`
/// fields; conversions are confined to compatibility boundaries.
#[derive(Debug, Clone, Copy)]
pub(crate) struct UsdPerMonthEstimate(Estimate);

/// Internal bytes/month estimate.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BytesPerMonthEstimate(Estimate);

/// Internal dimensionless multiplier, fraction, or execution count.
#[derive(Debug, Clone, Copy)]
pub(crate) struct UnitlessEstimate(Estimate);

impl UsdPerMonthEstimate {
    pub(crate) fn from_raw(value: Estimate) -> Self {
        Self(value)
    }

    pub(crate) fn raw(self) -> Estimate {
        self.0
    }

    pub(crate) fn median(self) -> f64 {
        self.0.median()
    }

    pub(crate) fn interval(self, coverage: f64) -> (f64, f64) {
        self.0.interval(coverage)
    }

    pub(crate) fn scaled_by(self, factor: UnitlessEstimate) -> Self {
        Self(self.0 * factor.0)
    }

    pub(crate) fn divided_by(self, divisor: UnitlessEstimate) -> Self {
        Self(self.0 / divisor.0)
    }

    pub(crate) fn positive_difference(self, other: Self, cv: f64) -> Self {
        Self(Estimate::from_point(
            (self.median() - other.median()).max(0.001),
            Some(cv),
        ))
    }

    pub(crate) fn efficiency_change(
        before: Self,
        after: Self,
        before_executions: UnitlessEstimate,
        after_executions: UnitlessEstimate,
        average_executions: UnitlessEstimate,
    ) -> Self {
        Self((before.0 / before_executions.0 - after.0 / after_executions.0) * average_executions.0)
    }

    pub(crate) fn volume_change(
        after: Self,
        after_executions: UnitlessEstimate,
        execution_delta: UnitlessEstimate,
    ) -> Self {
        Self(execution_delta.0 * (after.0 / after_executions.0))
    }
}

impl BytesPerMonthEstimate {
    pub(crate) fn from_raw(value: Estimate) -> Self {
        Self(value)
    }

    pub(crate) fn from_bytes_and_runs(bytes: Estimate, runs_per_month: Estimate) -> Self {
        Self(bytes * runs_per_month)
    }

    pub(crate) fn raw(self) -> Estimate {
        self.0
    }

    pub(crate) fn median(self) -> f64 {
        self.0.median()
    }

    pub(crate) fn scaled_by(self, factor: UnitlessEstimate) -> Self {
        Self(self.0 * factor.0)
    }

    pub(crate) fn divided_by(self, divisor: UnitlessEstimate) -> Self {
        Self(self.0 / divisor.0)
    }

    pub(crate) fn positive_difference(self, other: Self, cv: f64) -> Self {
        Self(Estimate::from_point(
            (self.median() - other.median()).max(0.001),
            Some(cv),
        ))
    }

    pub(crate) fn efficiency_change(
        before: Self,
        after: Self,
        before_executions: UnitlessEstimate,
        after_executions: UnitlessEstimate,
        average_executions: UnitlessEstimate,
    ) -> Self {
        Self((before.0 / before_executions.0 - after.0 / after_executions.0) * average_executions.0)
    }

    pub(crate) fn volume_change(
        after: Self,
        after_executions: UnitlessEstimate,
        execution_delta: UnitlessEstimate,
    ) -> Self {
        Self(execution_delta.0 * (after.0 / after_executions.0))
    }
}

impl UnitlessEstimate {
    pub(crate) fn from_raw(value: Estimate) -> Self {
        Self(value)
    }

    pub(crate) fn from_point(value: f64, cv: f64) -> Self {
        Self(Estimate::from_point(value, Some(cv)))
    }

    pub(crate) fn raw(self) -> Estimate {
        self.0
    }

    pub(crate) fn median(self) -> f64 {
        self.0.median()
    }
}

pub(crate) fn price_bytes(
    volume: BytesPerMonthEstimate,
    usd_per_byte: Estimate,
) -> UsdPerMonthEstimate {
    UsdPerMonthEstimate(volume.raw() * usd_per_byte)
}

pub(crate) fn sum_usd(values: &[UsdPerMonthEstimate]) -> Option<UsdPerMonthEstimate> {
    (!values.is_empty()).then(|| {
        UsdPerMonthEstimate(sum_lognormals(
            &values.iter().map(|value| value.raw()).collect::<Vec<_>>(),
        ))
    })
}

pub(crate) fn sum_bytes(values: &[BytesPerMonthEstimate]) -> Option<BytesPerMonthEstimate> {
    (!values.is_empty()).then(|| {
        BytesPerMonthEstimate(Estimate::from_point(
            values
                .iter()
                .map(|value| value.median())
                .sum::<f64>()
                .max(0.001),
            Some(0.01),
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pricing_is_the_only_bytes_to_usd_conversion() {
        let volume =
            BytesPerMonthEstimate::from_raw(Estimate::from_point(1_000_000_000_000.0, Some(0.1)));
        let price = Estimate::from_point(5.0 / 1_000_000_000_000.0, Some(0.05));
        let usd = price_bytes(volume, price);
        assert!((usd.median() - 5.0).abs() < 0.1);
    }

    #[test]
    fn sums_keep_units_separate() {
        let usd = [
            UsdPerMonthEstimate::from_raw(Estimate::from_point(2.0, Some(0.1))),
            UsdPerMonthEstimate::from_raw(Estimate::from_point(3.0, Some(0.1))),
        ];
        let bytes = [
            BytesPerMonthEstimate::from_raw(Estimate::from_point(20.0, Some(0.1))),
            BytesPerMonthEstimate::from_raw(Estimate::from_point(30.0, Some(0.1))),
        ];
        assert!((sum_usd(&usd).unwrap().median() - 5.0).abs() < 0.2);
        assert!((sum_bytes(&bytes).unwrap().median() - 50.0).abs() < 0.2);
    }
}
