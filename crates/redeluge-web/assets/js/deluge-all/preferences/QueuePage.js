/**
 * Deluge.preferences.QueuePage.js
 *
 * Copyright (c) Damien Churchill 2009-2010 <damoxc@gmail.com>
 *
 * This file is part of Deluge and is licensed under GNU General Public License 3.0, or later, with
 * the additional special exception to link portions of this program with the OpenSSL library.
 * See LICENSE for more details.
 */
Ext.namespace('Deluge.preferences');

/**
 * @class Deluge.preferences.Queue
 * @extends Ext.form.FormPanel
 */
Deluge.preferences.Queue = Ext.extend(Ext.form.FormPanel, {
    border: false,
    title: _('Queue'),
    header: false,
    layout: 'form',

    initComponent: function () {
        Deluge.preferences.Queue.superclass.initComponent.call(this);

        var om = deluge.preferences.getOptionsManager();

        var fieldset = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('New Torrents'),
            style: 'padding-top: 5px; margin-bottom: 0px;',
            autoHeight: true,
            labelWidth: 1,
            defaultType: 'checkbox',
        });
        om.bind(
            'queue_new_to_top',
            fieldset.add({
                fieldLabel: '',
                labelSeparator: '',
                boxLabel: _('Queue to top'),
                name: 'queue_new_to_top',
            })
        );

        fieldset = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('Active Torrents'),
            autoHeight: true,
            labelWidth: 150,
            defaultType: 'spinnerfield',
            style: 'padding-top: 5px; margin-bottom: 0px',
        });
        om.bind(
            'max_active_limit',
            fieldset.add({
                fieldLabel: _('Total:'),
                labelSeparator: '',
                name: 'max_active_limit',
                value: 8,
                width: 80,
                decimalPrecision: 0,
                minValue: -1,
                maxValue: 99999,
            })
        );
        om.bind(
            'max_active_downloading',
            fieldset.add({
                fieldLabel: _('Downloading:'),
                labelSeparator: '',
                name: 'max_active_downloading',
                value: 3,
                width: 80,
                decimalPrecision: 0,
                minValue: -1,
                maxValue: 99999,
            })
        );
        om.bind(
            'max_active_seeding',
            fieldset.add({
                fieldLabel: _('Seeding:'),
                labelSeparator: '',
                name: 'max_active_seeding',
                value: 5,
                width: 80,
                decimalPrecision: 0,
                minValue: -1,
                maxValue: 99999,
            })
        );
        om.bind(
            'dont_count_slow_torrents',
            fieldset.add({
                xtype: 'checkbox',
                name: 'dont_count_slow_torrents',
                hideLabel: true,
                boxLabel: _('Ignore slow torrents'),
            })
        );
        om.bind(
            'announce_to_all_tiers',
            fieldset.add({
                xtype: 'checkbox',
                name: 'announce_to_all_tiers',
                hideLabel: true,
                boxLabel: _('Announce to trackers in all tiers (one per tier)'),
            })
        );
        om.bind(
            'auto_manage_prefer_seeds',
            fieldset.add({
                xtype: 'checkbox',
                name: 'auto_manage_prefer_seeds',
                hideLabel: true,
                boxLabel: _('Prefer seeding torrents'),
            })
        );

        fieldset = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('Seeding Rotation'),
            autoHeight: true,
            labelWidth: 150,
            defaultType: 'spinnerfield',
            style: 'padding-top: 5px; margin-bottom: 0px',
        });
        om.bind(
            'share_ratio_limit',
            fieldset.add({
                fieldLabel: _('Share Ratio:'),
                labelSeparator: '',
                name: 'share_ratio_limit',
                value: 8,
                width: 80,
                incrementValue: 0.1,
                minValue: -1,
                maxValue: 99999,
                alternateIncrementValue: 1,
                decimalPrecision: 2,
            })
        );
        om.bind(
            'seed_time_ratio_limit',
            fieldset.add({
                fieldLabel: _('Time Ratio:'),
                labelSeparator: '',
                name: 'seed_time_ratio_limit',
                value: 3,
                width: 80,
                incrementValue: 0.1,
                minValue: -1,
                maxValue: 99999,
                alternateIncrementValue: 1,
                decimalPrecision: 2,
            })
        );
        om.bind(
            'seed_time_limit',
            fieldset.add({
                fieldLabel: _('Time (m):'),
                labelSeparator: '',
                name: 'seed_time_limit',
                value: 5,
                width: 80,
                decimalPrecision: 0,
                minValue: -1,
                maxValue: 99999,
            })
        );

        fieldset = this.add({
            xtype: 'fieldset',
            border: false,
            autoHeight: true,
            style: 'padding-top: 5px; margin-bottom: 0px',
            title: _('Share Ratio Reached'),

            layout: 'table',
            layoutConfig: { columns: 2 },
            labelWidth: 0,
            defaultType: 'checkbox',

            defaults: {
                fieldLabel: '',
                labelSeparator: '',
            },
        });
        this.stopAtRatio = fieldset.add({
            name: 'stop_seed_at_ratio',
            boxLabel: _('Share Ratio:'),
        });
        this.stopAtRatio.on('check', this.onStopRatioCheck, this);
        om.bind('stop_seed_at_ratio', this.stopAtRatio);

        this.stopRatio = fieldset.add({
            xtype: 'spinnerfield',
            name: 'stop_seed_ratio',
            ctCls: 'x-deluge-indent-checkbox',
            disabled: true,
            value: '2.0',
            width: 60,
            incrementValue: 0.1,
            minValue: -1,
            maxValue: 99999,
            alternateIncrementValue: 1,
            decimalPrecision: 2,
        });
        om.bind('stop_seed_ratio', this.stopRatio);

        this.removeAtRatio = fieldset.add({
            xtype: 'radiogroup',
            columns: 1,
            colspan: 2,
            disabled: true,
            style: 'margin-left: 10px',
            items: [
                {
                    boxLabel: _('Pause torrent'),
                    name: 'at_ratio',
                    inputValue: false,
                    checked: true,
                },
                {
                    boxLabel: _('Remove torrent'),
                    name: 'at_ratio',
                    inputValue: true,
                },
            ],
        });
        om.bind('remove_seed_at_ratio', this.removeAtRatio);

        // The idle rule. Its own fieldset with its own Apply, because it is
        // one dictionary under `idle_pause` rather than flat keys, and the
        // options manager above only binds flat ones.
        fieldset = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('Pause Idle Downloads'),
            autoHeight: true,
            labelWidth: 210,
            style: 'padding-top: 5px; margin-bottom: 0px',
        });
        fieldset.add({
            xtype: 'label',
            text: _(
                'A download that is transferring nothing still holds a place in the queue. This gives that place to a torrent that is waiting, and gives it back later.'
            ),
            style: 'display: block; margin-bottom: 6px; color: #666;',
        });

        this.idle = {};
        this.idle.enabled = fieldset.add({
            xtype: 'checkbox',
            hideLabel: true,
            boxLabel: _('Pause a download that gets nowhere'),
            handler: this.onIdleToggled,
            scope: this,
        });
        this.idle.inactive_rate = fieldset.add(
            this.idleSpinner(_('Idle below (KiB/s):'), 1, 0.1)
        );
        this.idle.grace = fieldset.add(
            this.idleSpinner(_('Idle for (minutes):'), 0)
        );
        this.idle.pause_for = fieldset.add(
            this.idleSpinner(_('Paused for (minutes):'), 0)
        );
        this.idle.min_active = fieldset.add(
            this.idleSpinner(_('Never leave fewer running than:'), 0)
        );
        this.idle.only_when_queued = fieldset.add({
            xtype: 'checkbox',
            hideLabel: true,
            boxLabel: _('Only when a torrent is waiting for the place'),
            ctCls: 'x-deluge-indent-checkbox',
        });

        // ------------------------------------ downloads that never get going
        fieldset = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('Downloads That Get Nowhere'),
            autoHeight: true,
            labelWidth: 210,
            style: 'padding-top: 5px; margin-bottom: 0px',
        });
        fieldset.add({
            xtype: 'label',
            text: _(
                'A torrent that has been trying for hours without a single byte arriving is not slow, it is dead: a magnet nobody seeds, or content that has left the swarm. A label can set its own rule, or turn this off for its torrents. Nothing is acted on while its tracker is failing every announce — an outage is not a dead swarm.'
            ),
            style: 'display: block; margin-bottom: 6px; color: #666;',
        });

        this.stuck = {};
        this.stuck.enabled = fieldset.add({
            xtype: 'checkbox',
            hideLabel: true,
            boxLabel: _('Act on a download that stops getting anywhere'),
            handler: this.onStuckToggled,
            scope: this,
        });
        this.stuck.hours = fieldset.add(
            this.idleSpinner(_('Nothing arriving for (hours):'), 1)
        );
        this.stuck.max_progress = fieldset.add(
            this.idleSpinner(_('And no further along than (%):'), 0)
        );
        fieldset.add({
            xtype: 'label',
            text: _(
                'Zero per cent takes only what never started, which is the safe reading and the default. Raising it puts torrents that did start and then stalled in scope — at a hundred, one stalled at 90% goes the same way. The hours are counted in time spent trying, so a torrent that sat in the queue or was paused overnight has not been failing for a night.'
            ),
            style: 'display: block; margin: 2px 0 6px 0; color: #666;',
        });
        this.stuck.action = fieldset.add({
            xtype: 'combo',
            fieldLabel: _('Then:'),
            labelSeparator: '',
            width: 220,
            mode: 'local',
            triggerAction: 'all',
            editable: false,
            valueField: 'id',
            displayField: 'text',
            value: 'pause',
            store: new Ext.data.ArrayStore({
                idIndex: 0,
                fields: ['id', 'text'],
                data: [
                    ['pause', _('Pause it and leave it alone')],
                    ['remove', _('Remove it')],
                ],
            }),
            listeners: { select: this.onStuckToggled, scope: this },
        });
        this.stuck.label = fieldset.add({
            xtype: 'textfield',
            fieldLabel: _('Put it in label:'),
            labelSeparator: '',
            width: 160,
            emptyText: _('leave its label alone'),
        });
        fieldset.add({
            xtype: 'label',
            text: _(
                'A label whose own rule is off is an exemption, so filing paused torrents in one is how they stop being looked at every minute — and how you find them again to decide.'
            ),
            style: 'display: block; margin: 2px 0 6px 0; color: #666;',
        });
        this.stuck.remove_data = fieldset.add({
            xtype: 'checkbox',
            hideLabel: true,
            boxLabel: _('Delete their files as well'),
            ctCls: 'x-deluge-indent-checkbox',
            handler: this.onStuckToggled,
            scope: this,
        });
        this.stuckWarning = fieldset.add({
            xtype: 'label',
            hidden: true,
            text: _(
                'The files will be deleted from disk. There is no undo, and nothing else is asked first.'
            ),
            style: 'display: block; margin: 2px 0 0 18px; color: #a03030;',
        });

        this.on('show', this.onPageShow, this);
    },

    /**
     * The rule needs four numbers, and they are all shaped the same.
     */
    idleSpinner: function (caption, precision, increment) {
        return {
            xtype: 'spinnerfield',
            fieldLabel: caption,
            labelSeparator: '',
            width: 80,
            decimalPrecision: precision,
            minValue: 0,
            maxValue: 99999,
            incrementValue: increment || 1,
        };
    },

    onPageShow: function () {
        this.loadStuck();
        if (this.idleLoaded) return;
        this.idleLoaded = true;
        deluge.client.core.get_config_value('idle_pause', {
            success: function (settings) {
                settings = settings || {};
                this.idle.enabled.setValue(settings['enabled'] === true);
                // Stored in bytes per second and in seconds, shown in the
                // units the rest of this window uses.
                this.idle.inactive_rate.setValue(
                    Ext.value(settings['inactive_rate'], 2048) / 1024
                );
                this.idle.grace.setValue(
                    Math.round(Ext.value(settings['grace'], 300) / 60)
                );
                this.idle.pause_for.setValue(
                    Math.round(Ext.value(settings['pause_for'], 3600) / 60)
                );
                this.idle.min_active.setValue(
                    Ext.value(settings['min_active'], 1)
                );
                this.idle.only_when_queued.setValue(
                    settings['only_when_queued'] !== false
                );
                this.onIdleToggled();
            },
            failure: function () {
                this.onIdleToggled();
            },
            scope: this,
        });
    },

    /**
     * The rule's numbers mean nothing while it is off, so they follow it.
     */
    onIdleToggled: function () {
        var on = this.idle.enabled.getValue() === true;
        Ext.each(
            [
                'inactive_rate',
                'grace',
                'pause_for',
                'min_active',
                'only_when_queued',
            ],
            function (name) {
                this.idle[name].setDisabled(!on);
            },
            this
        );
    },

    onApply: function () {
        // Nothing read yet means nothing of this page's to write. The other
        // feature pages learned this the hard way: Preferences applies every
        // page on OK, and an unread page holds defaults. Each block guards
        // itself, because one that failed to read must not ride out on the
        // other one having succeeded.
        if (this.stuckLoaded) {
            deluge.client.core.set_config({
                stuck: {
                    enabled: this.stuck.enabled.getValue() === true,
                    hours: Deluge.number(this.stuck.hours.getValue(), 0),
                    max_progress: Deluge.number(
                        this.stuck.max_progress.getValue(),
                        0
                    ),
                    action: this.stuck.action.getValue() || 'pause',
                    label: this.stuck.label.getValue() || '',
                    remove_data: this.stuck.remove_data.getValue() === true,
                },
            });
        }

        if (!this.idleLoaded) return;

        deluge.client.core.set_config({
            idle_pause: {
                enabled: this.idle.enabled.getValue() === true,
                inactive_rate: Math.round(
                    Deluge.number(this.idle.inactive_rate.getValue(), 2) * 1024
                ),
                grace: Deluge.number(this.idle.grace.getValue(), 5) * 60,
                pause_for: Deluge.number(this.idle.pause_for.getValue(), 60) * 60,
                min_active: Deluge.number(this.idle.min_active.getValue(), 1),
                only_when_queued:
                    this.idle.only_when_queued.getValue() === true,
            },
        });
    },

    onStopRatioCheck: function (e, checked) {
        this.stopRatio.setDisabled(!checked);
        this.removeAtRatio.setDisabled(!checked);
    },

    /**
     * Reads the daemon's own stuck rule. Its own call, because it is a feature
     * key rather than one of the settings the options manager carries.
     */
    loadStuck: function () {
        if (this.stuckLoaded) return;
        this.stuckLoaded = true;
        deluge.client.core.get_config_value('stuck', {
            success: function (settings) {
                settings = settings || {};
                this.stuck.enabled.setValue(settings['enabled'] === true);
                this.stuck.hours.setValue(Deluge.number(settings['hours'], 0));
                this.stuck.max_progress.setValue(
                    Deluge.number(settings['max_progress'], 0)
                );
                this.stuck.action.setValue(settings['action'] || 'pause');
                this.stuck.label.setValue(settings['label'] || '');
                this.stuck.remove_data.setValue(
                    settings['remove_data'] !== false
                );
                this.onStuckToggled();
            },
            failure: function () {
                // The daemon may not be connected. Leaving the boxes at their
                // defaults is right; writing them back is not, which is what
                // `stuckLoaded` guards on the way out.
                this.stuckLoaded = false;
            },
            scope: this,
        });
    },

    /**
     * The numbers mean nothing until the switch is on, and the warning is only
     * worth reading while the thing it warns about is armed.
     */
    onStuckToggled: function () {
        var on = this.stuck.enabled.getValue() === true;
        var removing = this.stuck.action.getValue() === 'remove';
        this.stuck.hours.setDisabled(!on);
        this.stuck.max_progress.setDisabled(!on);
        this.stuck.action.setDisabled(!on);
        // Each half belongs to one action and means nothing under the other.
        this.stuck.label.setDisabled(!on || removing);
        this.stuck.remove_data.setDisabled(!on || !removing);
        this.stuckWarning.setVisible(
            on && removing && this.stuck.remove_data.getValue() === true
        );
    },
});
