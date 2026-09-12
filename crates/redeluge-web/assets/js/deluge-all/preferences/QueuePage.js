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
        // page on OK, and an unread page holds defaults.
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
});
