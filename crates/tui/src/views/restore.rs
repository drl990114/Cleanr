use super::*;

pub(crate) fn render_restore(frame: &mut Frame<'_>, area: Rect, app: &mut Workbench) {
    let restored = restored_run_ids(&app.restore_manifests)
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let mut details = Vec::new();
    let mut more = vec![detail_line(
        &app.i18n.t("detail_manifest_dir"),
        app.state_dir.display().to_string(),
        app.theme.fg_dim,
        app.theme,
    )];
    if let Some(manifest) = app
        .list_state
        .selected()
        .and_then(|i| app.execution_manifests.get(i))
    {
        details.push(home_title(
            manifest
                .created_at
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string(),
            app.theme,
        ));
        details.push(Line::from(app.i18n.format(
            "restore_item_count",
            &[("count", manifest.summary.succeeded.to_string())],
        )));
        details.push(Line::from(app.i18n.t(
            if restored.contains(manifest.run_id.as_str()) {
                "restore_state_restored"
            } else {
                "restore_state_available"
            },
        )));
        if manifest.summary.failed > 0 {
            details.push(Line::from(Span::styled(
                app.i18n.format(
                    "cleanup_result_failed",
                    &[("count", manifest.summary.failed.to_string())],
                ),
                Style::default().fg(app.theme.warn),
            )));
        }
        more.extend([
            detail_line(
                &app.i18n.t("detail_run"),
                manifest.run_id.clone(),
                app.theme.fg_dim,
                app.theme,
            ),
            detail_line(
                &app.i18n.t("detail_created"),
                manifest.created_at.to_rfc3339(),
                app.theme.fg_dim,
                app.theme,
            ),
        ]);
    } else {
        details.push(Line::from(app.i18n.t("status_no_manifests")));
    }
    let max_count = app
        .execution_manifests
        .iter()
        .map(|m| m.summary.succeeded)
        .max()
        .unwrap_or(0);
    let count_width = display_width(
        &app.i18n
            .format("restore_item_count", &[("count", max_count.to_string())]),
    ) as u16;
    let content = ContextContent {
        title: app.i18n.t("label_restore"),
        count: app.execution_manifests.len(),
        columns: vec![
            Constraint::Fill(1),
            Constraint::Length(count_width),
            Constraint::Length(10),
        ],
        empty: app.i18n.t("status_no_manifests"),
        details,
        more,
    };
    render_context_workspace(frame, area, app, content, |app, window, widths| {
        app.execution_manifests[window]
            .iter()
            .map(|manifest| {
                let was_restored = restored.contains(manifest.run_id.as_str());
                Row::new(vec![
                    text_cell(
                        manifest
                            .created_at
                            .with_timezone(&chrono::Local)
                            .format(if widths[0] >= 16 {
                                "%Y-%m-%d %H:%M"
                            } else {
                                "%m-%d %H:%M"
                            })
                            .to_string(),
                        widths[0],
                        app.theme.fg,
                    ),
                    right_cell(
                        app.i18n.format(
                            "restore_item_count",
                            &[("count", manifest.summary.succeeded.to_string())],
                        ),
                        app.theme.fg,
                    ),
                    text_cell(
                        app.i18n.t(if was_restored {
                            "restore_state_restored"
                        } else {
                            "restore_state_available"
                        }),
                        widths[2],
                        if was_restored {
                            app.theme.ok
                        } else {
                            app.theme.fg_dim
                        },
                    ),
                ])
            })
            .collect()
    });
}
