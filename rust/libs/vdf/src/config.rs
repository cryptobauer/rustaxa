pub struct VdfConfig {
    pub difficulty_min: u16,
    pub difficulty_max: u16,
    pub difficulty_stale: u16,
    pub lambda_bound: u16,
}

impl VdfConfig {
    pub fn new(
        difficulty_min: u16,
        difficulty_max: u16,
        difficulty_stale: u16,
        lambda_bound: u16,
    ) -> Self {
        Self {
            difficulty_min,
            difficulty_max,
            difficulty_stale,
            lambda_bound,
        }
    }

    pub fn get_default() -> Self {
        Self {
            difficulty_min: 0,
            difficulty_max: 1,
            difficulty_stale: 0,
            lambda_bound: 1500,
        }
    }
}

// pub struct SortitionParams {
//     pub vrf: VrfConfig,
//     pub vdf: VdfConfig,
// }

// impl SortitionParams {
//     pub fn new(vrf: VrfConfig, vdf: VdfConfig) -> Self {
//         Self { vrf, vdf }
//     }

//     pub fn default() -> Self {
//         Self {
//             vrf: VrfConfig::default(),
//             vdf: VdfConfig::default(),
//         }
//     }
// }

// pub struct SortitionConfig {
//     pub params: SortitionParams,
//     pub changes_count_for_average: u16,
//     pub dag_efficiency_targets: (u16, u16),
//     pub changing_interval: u16,
//     pub computation_interval: u16,
// }

// impl SortitionConfig {
//     pub fn new(
//         params: SortitionParams,
//         changes_count_for_average: u16,
//         dag_efficiency_targets: (u16, u16),
//         changing_interval: u16,
//         computation_interval: u16,
//     ) -> Self {
//         Self {
//             params,
//             changes_count_for_average,
//             dag_efficiency_targets,
//             changing_interval,
//             computation_interval,
//         }
//     }

//     pub fn default() -> Self {
//         Self {
//             params: SortitionParams::default(),
//             changes_count_for_average: 10,
//             dag_efficiency_targets: (69 * 100, 71 * 100),
//             changing_interval: 200,
//             computation_interval: 50,
//         }
//     }

//     pub fn target_efficiency(&self) -> u16 {
//         (self.dag_efficiency_targets.0 + self.dag_efficiency_targets.1) / 2
//     }
// }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vdf_config_new() {
        let config = VdfConfig::new(5, 20, 10, 1000);
        assert_eq!(config.difficulty_min, 5);
        assert_eq!(config.difficulty_max, 20);
        assert_eq!(config.difficulty_stale, 10);
        assert_eq!(config.lambda_bound, 1000);
    }

    #[test]
    fn test_vdf_config_default() {
        let config = VdfConfig::get_default();
        assert_eq!(config.difficulty_min, 0);
        assert_eq!(config.difficulty_max, 1);
        assert_eq!(config.difficulty_stale, 0);
        assert_eq!(config.lambda_bound, 1500);
    }

    // #[test]
    // fn test_sortition_params_new() {
    //     let vrf = VrfConfig::new(150);
    //     let vdf = VdfConfig::new(3, 15, 8, 2500);
    //     let params = SortitionParams::new(vrf, vdf);

    //     assert_eq!(params.vrf.threshold_upper, 150);
    //     assert_eq!(params.vdf.difficulty_min, 3);
    //     assert_eq!(params.vdf.difficulty_max, 15);
    //     assert_eq!(params.vdf.difficulty_stale, 8);
    //     assert_eq!(params.vdf.lambda_bound, 2500);
    // }

    // #[test]
    // fn test_sortition_params_default() {
    //     let params = SortitionParams::default();
    //     assert_eq!(params.vrf.threshold_upper, 0);
    //     assert_eq!(params.vdf.difficulty_min, 0);
    //     assert_eq!(params.vdf.difficulty_max, 1);
    //     assert_eq!(params.vdf.difficulty_stale, 0);
    //     assert_eq!(params.vdf.lambda_bound, 1500);
    // }

    // #[test]
    // fn test_sortition_config_default() {
    //     let config = SortitionConfig::default();
    //     assert_eq!(config.params.vrf.threshold_upper, 0);
    //     assert_eq!(config.params.vdf.difficulty_min, 0);
    //     assert_eq!(config.params.vdf.difficulty_max, 1);
    //     assert_eq!(config.params.vdf.difficulty_stale, 0);
    //     assert_eq!(config.params.vdf.lambda_bound, 1500);
    //     assert_eq!(config.changing_interval, 200);
    //     assert_eq!(config.computation_interval, 50);
    //     assert_eq!(config.changes_count_for_average, 10);
    //     assert_eq!(config.dag_efficiency_targets, (6900, 7100));
    //     assert_eq!(config.target_efficiency(), 7000);
    // }
}
