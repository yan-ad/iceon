//! Lightweight status and metadata badge component for resource tables, inspector cards, and pod dock.

use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeVariant {
    Success,
    Warning,
    Danger,
    Secondary,
    Outline,
}

#[derive(IntoElement)]
pub struct Badge {
    pub(crate) variant: BadgeVariant,
    small: bool,
    show_dot: bool,
    label: SharedString,
}

impl Badge {
    pub fn new(variant: BadgeVariant) -> Self {
        Self {
            variant,
            small: false,
            show_dot: matches!(
                variant,
                BadgeVariant::Success | BadgeVariant::Warning | BadgeVariant::Danger
            ),
            label: SharedString::default(),
        }
    }

    pub fn success() -> Self {
        Self::new(BadgeVariant::Success)
    }

    pub fn warning() -> Self {
        Self::new(BadgeVariant::Warning)
    }

    pub fn danger() -> Self {
        Self::new(BadgeVariant::Danger)
    }

    pub fn secondary() -> Self {
        Self::new(BadgeVariant::Secondary)
    }

    pub fn outline() -> Self {
        Self::new(BadgeVariant::Outline)
    }

    pub fn small(mut self) -> Self {
        self.small = true;
        self
    }

    pub fn without_icon(mut self) -> Self {
        self.show_dot = false;
        self
    }

    pub fn child(mut self, text: impl Into<SharedString>) -> Self {
        self.label = text.into();
        self
    }

    pub fn for_resource_status(status: &str) -> Self {
        let normalized = status
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase();
        let (label, condition_status) = normalized
            .split_once([':', '='])
            .map_or((normalized.as_str(), None), |(label, value)| {
                (label.trim(), Some(value.trim()))
            });
        // Bare condition types represent the API's explicitly true conditions.
        // A false/unknown (or malformed) value must never inherit that type's tone.
        let variant = match condition_status {
            Some("false") => BadgeVariant::Warning,
            Some(value) if value != "true" => BadgeVariant::Secondary,
            _ => {
                // Failure signals win even in compound labels such as Running (Error).
                // Only failures use substring detection; positive labels are a closed
                // vocabulary, so negated or unfamiliar observations cannot look healthy.
                if label.contains("fail")
                    || label.contains("error")
                    || label.contains("crash")
                    || matches!(
                        label,
                        "not ready" | "notready" | "lost" | "blocked" | "false" | "degraded"
                    )
                {
                    BadgeVariant::Danger
                } else {
                    match label {
                        // Observed phases and affirmative health/condition labels.
                        "running"
                        | "ready"
                        | "active"
                        | "bound"
                        | "complete"
                        | "completed"
                        | "succeeded"
                        | "deployed"
                        | "available"
                        | "healthy"
                        | "normal"
                        | "true"
                        | "abletoscale"
                        | "scalingactive"
                        | "established"
                        | "namesaccepted"
                        | "disruptionallowed"
                        | "recommendationprovided" => BadgeVariant::Success,
                        // Transitional, inhibited, or explicitly negative observations.
                        "pending" | "terminating" | "warning" | "waiting" | "progressing"
                        | "scaling" | "suspended" | "schedulingdisabled" | "scalinglimited"
                        | "fetchinghistory" | "lowconfidence" | "nopodsmatched" | "paused"
                        | "inactive" | "unavailable" | "incomplete" => BadgeVariant::Warning,
                        // Missing, unknown, non-applicable, and unrecognized labels are
                        // neutral: absence of a failure is not evidence of health.
                        _ => BadgeVariant::Secondary,
                    }
                }
            }
        };
        Self::new(variant).small().child(status.to_owned())
    }
}

impl RenderOnce for Badge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_dark = cx.theme().mode.is_dark();
        let (bg, fg, border, dot): (Hsla, Hsla, Hsla, Hsla) = match self.variant {
            BadgeVariant::Success => {
                let c = cx.theme().success;
                (
                    c.alpha(if is_dark { 0.16 } else { 0.12 }),
                    c,
                    c.alpha(0.32),
                    c,
                )
            }
            BadgeVariant::Warning => {
                let c = cx.theme().warning;
                (
                    c.alpha(if is_dark { 0.16 } else { 0.12 }),
                    c,
                    c.alpha(0.32),
                    c,
                )
            }
            BadgeVariant::Danger => {
                let c = cx.theme().danger;
                (
                    c.alpha(if is_dark { 0.16 } else { 0.12 }),
                    c,
                    c.alpha(0.32),
                    c,
                )
            }
            BadgeVariant::Secondary => (
                cx.theme().secondary,
                cx.theme().muted_foreground,
                cx.theme().border,
                cx.theme().muted_foreground,
            ),
            BadgeVariant::Outline => (
                gpui_kit::transparent_black(),
                cx.theme().muted_foreground,
                cx.theme().border,
                cx.theme().muted_foreground,
            ),
        };

        h_flex()
            .items_center()
            .gap_1()
            .px(if self.small { px(6.0) } else { px(8.0) })
            .py(if self.small { px(0.5) } else { px(2.5) })
            .rounded(px(if self.small { 3.0 } else { 5.0 }))
            .border_1()
            .border_color(border)
            .bg(bg)
            .text_xs()
            .when(self.small, |el| {
                el.text_size(px(11.0)).line_height(px(14.0))
            })
            .text_color(fg)
            .when(self.show_dot, |el| {
                el.child(
                    div()
                        .size(px(if self.small { 4.0 } else { 5.0 }))
                        .flex_shrink_0()
                        .rounded_full()
                        .bg(dot),
                )
            })
            .child(self.label)
    }
}

#[cfg(test)]
mod tests {
    use super::{Badge, BadgeVariant};

    #[test]
    fn resource_status_badges_use_the_compact_size_without_changing_labels() {
        for label in ["Active", "Terminating", "Not reported", "Unknown"] {
            let badge = Badge::for_resource_status(label);
            assert!(badge.small);
            assert_eq!(badge.label.as_ref(), label);
        }
    }

    #[test]
    fn known_positive_statuses_match_complete_normalized_labels() {
        for label in [
            "Running",
            "Ready",
            "Active",
            "Bound",
            "Complete",
            "Completed",
            "Succeeded",
            "Deployed",
            "Available",
            "Healthy",
            "Normal",
            "True",
            "AbleToScale",
            "ScalingActive",
            "Established",
            "NamesAccepted",
            "DisruptionAllowed",
            "RecommendationProvided",
            "  rEaDy  ",
            " Available : True ",
        ] {
            assert_eq!(
                Badge::for_resource_status(label).variant,
                BadgeVariant::Success,
                "{label}"
            );
        }
    }

    #[test]
    fn negative_and_transitional_labels_are_not_healthy() {
        for label in [
            "Not ready",
            "NotReady",
            "  NOT   READY  ",
            "Failed",
            "CrashLoopBackOff",
            "Running (Error)",
            "Ready but failed",
            "ReplicaFailure",
            "Lost",
            "Blocked",
            "False",
        ] {
            assert_eq!(
                Badge::for_resource_status(label).variant,
                BadgeVariant::Danger,
                "{label}"
            );
        }
        for label in [
            "Inactive",
            "Unavailable",
            "incomplete",
            "Progressing",
            "Pending",
            "Waiting",
            "Scaling",
            "Suspended",
            "Terminating",
            "SchedulingDisabled",
            "ScalingLimited",
            "LowConfidence",
            "NoPodsMatched",
            "Ready: False",
            "Available = false",
        ] {
            assert_eq!(
                Badge::for_resource_status(label).variant,
                BadgeVariant::Warning,
                "{label}"
            );
        }
    }

    #[test]
    fn missing_unknown_and_arbitrary_labels_are_neutral() {
        for label in [
            "",
            "  ",
            "Unknown",
            "Not reported",
            "Not applicable",
            "Not   reported",
            "Ready: Unknown",
            "Active: Unknown",
            "Available: ",
            "Ready: maybe",
            "not running",
            "Already",
            "Unbound",
            "not healthy",
            "future active state",
            "Completed-ish",
        ] {
            assert_eq!(
                Badge::for_resource_status(label).variant,
                BadgeVariant::Secondary,
                "{label}"
            );
        }
    }
}
