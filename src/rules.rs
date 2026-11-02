pub(crate) fn click_damage(base: u32, bonus: u32) -> u32 {
    base.saturating_add(bonus)
}

pub(crate) fn health_after_hit(health: u32, damage: u32) -> u32 {
    health.saturating_sub(damage)
}

pub(crate) fn is_defeated(health: u32) -> bool {
    health == 0
}

pub(crate) fn enemy_health_for_stage(stage: u32) -> u32 {
    stage.saturating_mul(10).saturating_add(10)
}

pub(crate) fn gold_reward_for_stage(stage: u32) -> u32 {
    stage.saturating_mul(2).saturating_add(3)
}

pub(crate) fn xp_reward_for_stage(stage: u32) -> u32 {
    stage.saturating_mul(2).saturating_add(4)
}

pub(crate) fn xp_to_next_level(level: u32) -> u32 {
    level
        .saturating_sub(1)
        .saturating_mul(15)
        .saturating_add(20)
}

pub(crate) fn base_damage_for_level(level: u32) -> u32 {
    level.saturating_add(4)
}

pub(crate) fn idle_damage_for_level(level: u32) -> u32 {
    level
}

pub(crate) fn click_upgrade_bonus(upgrades: u32) -> u32 {
    upgrades.saturating_mul(2)
}

pub(crate) fn click_upgrade_cost(upgrades: u32) -> u32 {
    upgrades.saturating_add(1).saturating_mul(10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_damage_adds_base_and_bonus() {
        assert_eq!(click_damage(3, 4), 7);
    }

    #[test]
    fn health_never_drops_below_zero() {
        assert_eq!(health_after_hit(3, 10), 0);
    }

    #[test]
    fn zero_health_is_defeated() {
        assert!(is_defeated(0));
        assert!(!is_defeated(1));
    }

    #[test]
    fn later_stages_have_tougher_enemies() {
        assert_eq!(enemy_health_for_stage(1), 20);
        assert_eq!(enemy_health_for_stage(2), 30);
        // saturating_mul/add keep extreme stages from overflowing u32.
        assert_eq!(enemy_health_for_stage(u32::MAX), u32::MAX);
    }

    #[test]
    fn later_stages_pay_better() {
        assert_eq!(gold_reward_for_stage(1), 5);
        assert_eq!(gold_reward_for_stage(2), 7);
        assert_eq!(xp_reward_for_stage(1), 6);
        assert_eq!(xp_reward_for_stage(2), 8);
    }

    #[test]
    fn levels_need_more_xp_as_they_go() {
        assert_eq!(xp_to_next_level(1), 20);
        assert_eq!(xp_to_next_level(2), 35);
        assert_eq!(xp_to_next_level(3), 50);
    }

    #[test]
    fn levels_make_the_player_hit_harder() {
        assert_eq!(base_damage_for_level(1), 5);
        assert_eq!(base_damage_for_level(2), 6);
    }

    #[test]
    fn idle_damage_grows_with_level() {
        assert_eq!(idle_damage_for_level(1), 1);
        assert_eq!(idle_damage_for_level(2), 2);
    }

    #[test]
    fn upgrades_cost_more_each_time() {
        assert_eq!(click_upgrade_cost(0), 10);
        assert_eq!(click_upgrade_cost(1), 20);
    }
}
