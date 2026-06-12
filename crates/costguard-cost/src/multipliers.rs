use crate::config::{range_or_point_to_estimate, CostConfig};
use crate::Estimate;
use std::collections::HashMap;
use std::sync::OnceLock;

pub fn rule_multiplier(rule_id: &str, config: &CostConfig) -> Option<Estimate> {
    if is_infrastructure_rule(rule_id) {
        return None;
    }
    if let Some(override_cfg) = config.rules.get(rule_id) {
        if let Some(mult) = &override_cfg.multiplier {
            return Some(range_or_point_to_estimate(mult, Some(0.2)));
        }
    }
    default_multipliers().get(rule_id).copied()
}

fn is_infrastructure_rule(rule_id: &str) -> bool {
    matches!(
        rule_id,
        "SQLCOST023" | "SQLCOST024" | "SQLCOST025" | "SQLCOST026" | "SQLCOST027"
    )
}

fn mult(p10: f64, p90: f64) -> Estimate {
    Estimate::from_range(p10, p90)
}

fn default_multipliers() -> &'static HashMap<&'static str, Estimate> {
    static MULTIPLIERS: OnceLock<HashMap<&'static str, Estimate>> = OnceLock::new();
    MULTIPLIERS.get_or_init(|| {
        let mut m = HashMap::new();
        m.insert("SQLCOST001", mult(1.2, 2.0));
        m.insert("SQLCOST002", mult(1.5, 3.0));
        m.insert("SQLCOST003", mult(1.2, 2.5));
        m.insert("SQLCOST004", mult(1.5, 4.0));
        m.insert("SQLCOST005", mult(2.0, 8.0));
        m.insert("SQLCOST006", mult(3.0, 20.0));
        m.insert("SQLCOST007", mult(1.5, 4.0));
        m.insert("SQLCOST008", mult(1.5, 3.0));
        m.insert("SQLCOST009", mult(1.2, 2.0));
        m.insert("SQLCOST010", mult(1.2, 2.0));
        m.insert("SQLCOST011", mult(1.5, 3.0));
        m.insert("SQLCOST012", mult(5.0, 50.0));
        m.insert("SQLCOST013", mult(1.5, 3.0));
        m.insert("SQLCOST014", mult(2.0, 6.0));
        m.insert("SQLCOST015", mult(1.5, 4.0));
        m.insert("SQLCOST016", mult(2.0, 10.0));
        m.insert("SQLCOST017", mult(1.5, 4.0));
        m.insert("SQLCOST018", mult(1.5, 3.0));
        m.insert("SQLCOST019", mult(2.0, 8.0));
        m.insert("SQLCOST020", mult(1.5, 3.0));
        m.insert("SQLCOST021", mult(1.2, 2.0));
        m.insert("SQLCOST022", mult(1.2, 2.0));
        m.insert("SQLCOST028", mult(5.0, 100.0));
        m.insert("SQLCOST029", mult(2.0, 10.0));
        m.insert("SQLCOST030", mult(2.0, 8.0));
        m.insert("SQLCOST031", mult(3.0, 20.0));
        m.insert("SQLCOST032", mult(3.0, 30.0));
        m.insert("SQLCOST033", mult(2.0, 10.0));
        m.insert("SQLCOST034", mult(1.5, 5.0));
        m.insert("SQLCOST035", mult(2.0, 8.0));
        m
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_infrastructure_rules() {
        let config = CostConfig::default();
        assert!(rule_multiplier("SQLCOST023", &config).is_none());
        assert!(rule_multiplier("SQLCOST014", &config).is_some());
    }
}
