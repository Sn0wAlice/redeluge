/**
 * Deluge.TrackerSettingsWindow.js
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 *
 * What the daemon should do with the torrents of one tracker.
 *
 * A tracker is not something you create, the way a label is: it is whatever
 * the torrents you added announce to, and the sidebar has been grouping them
 * by it all along. This window is reached by right-clicking one of those rows,
 * because that row is the thing the rules belong to.
 *
 * Four rules so far: hold these torrents to a set of limits, label them, move
 * them when they are done, and remove them when they are done with. The window is built from
 * `Deluge.TrackerSettingsWindow.RULES` rather than written out, so a fourth is
 * an entry in that list and the daemon side that acts on it, and nothing else
 * in this file changes. Every rule is a switch and the fields that switch
 * governs, which is the shape the label options already have, so the two
 * windows behave the same way.
 *
 * The settings live under the `tracker` key of the daemon's configuration, so
 * `core.get_config` and `core.set_config` are the whole interface and nothing
 * new had to be added to the RPC for them.
 */
Ext.ns('Deluge');

/**
 * @class Deluge.TrackerSettingsWindow
 * @extends Ext.Window
 */
Deluge.TrackerSettingsWindow = Ext.extend(Ext.Window, {
    title: _('Tracker Settings'),
    width: 520,
    height: 660,
    layout: 'fit',
    buttonAlign: 'right',
    closeAction: 'hide',
    constrainHeader: true,
    plain: true,
    minWidth: 380,
    minHeight: 260,

    initComponent: function () {
        // What the form in front of you would do, kept beside the OK button
        // rather than inside the scrolling form: the one moment it matters is
        // the moment before somebody presses OK on a rule that deletes things.
        // Set before the superclass runs, which is when a panel turns `bbar`
        // into a toolbar; adding it afterwards would make it a second item of
        // a `fit` layout, which draws one thing.
        this.preview = new Ext.Toolbar.TextItem({ text: '' });
        this.bbar = [this.preview];

        Deluge.TrackerSettingsWindow.superclass.initComponent.call(this);

        this.addButton(_('Cancel'), this.onCancel, this);
        this.addButton(_('OK'), this.onOk, this);

        this.form = this.add({
            xtype: 'form',
            border: false,
            autoScroll: true,
            bodyStyle: 'padding: 5px',
            labelWidth: 150,
        });

        this.form.add({
            xtype: 'label',
            text: _(
                'These apply to every torrent that announces to this tracker, whenever it was added. Each is off until you turn it on, and no other tracker is affected.'
            ),
            style: 'display: block; margin: 0 4px 6px 4px; opacity: 0.72;',
        });

        // Every field in the window by its setting name, and the switches by
        // the group they govern. Built from RULES so that adding a rule is
        // adding a rule, not editing five functions.
        this.fields = {};
        this.defaults = {};
        this.groups = [];
        Ext.each(Deluge.TrackerSettingsWindow.RULES, this.addRule, this);

    },

    /**
     * One rule: its switch, its fields, and whatever it has to warn about.
     */
    addRule: function (rule) {
        var set = this.form.add({
            xtype: 'fieldset',
            border: false,
            title: rule.title,
            autoHeight: true,
            labelWidth: 150,
            style: 'padding-top: 5px; margin-bottom: 0px;',
        });

        var group = { key: rule.key, fields: [], warnings: [] };

        this.fields[rule.key] = set.add({
            xtype: 'checkbox',
            hideLabel: true,
            boxLabel: rule.boxLabel,
            handler: this.onSwitched,
            scope: this,
        });
        this.defaults[rule.key] = false;

        Ext.each(
            rule.fields,
            function (spec) {
                var config = Ext.apply({}, spec);
                delete config.name;
                delete config.warn;
                delete config.dflt;
                delete config.needs;
                // Indented under the switch that governs them, which is how
                // the label options draw the same relationship.
                config.ctCls = 'x-deluge-indent-checkbox';
                if (config.xtype === 'checkbox') {
                    config.handler = this.onSwitched;
                    config.scope = this;
                }
                if (config.xtype === 'spinnerfield') {
                    config.listeners = {
                        spin: { fn: this.describe, scope: this },
                        change: { fn: this.describe, scope: this },
                    };
                }
                if (config.xtype === 'textfield') {
                    config.listeners = {
                        change: { fn: this.describe, scope: this },
                    };
                }

                var field = set.add(config);
                this.fields[spec.name] = field;
                // What this setting is when the tracker's entry does not
                // carry it, which has to be the daemon's default and not a
                // blank: an entry written by an older build, or by hand, has
                // only the keys somebody set.
                this.defaults[spec.name] = this.defaultOf(spec);
                group.fields.push({ name: spec.name, needs: spec.needs });

                if (spec.warn) {
                    group.warnings.push({
                        field: spec.name,
                        label: set.add({
                            xtype: 'label',
                            hidden: true,
                            text: spec.warn,
                            style:
                                'display: block; margin: 2px 0 0 18px; color: #e6381f;',
                        }),
                    });
                }
            },
            this
        );

        if (rule.note) {
            set.add({
                xtype: 'label',
                text: rule.note,
                style: 'display: block; margin: 4px 0 0 0; opacity: 0.72;',
            });
        }

        this.groups.push(group);
    },

    /**
     * What a field is worth when the tracker's entry says nothing about it.
     */
    defaultOf: function (spec) {
        if (spec.dflt !== undefined) return spec.dflt;
        if (spec.xtype === 'checkbox') return false;
        if (spec.xtype === 'spinnerfield') return spec.value || 0;
        return '';
    },

    /**
     * Opens the window on one tracker.
     *
     * @param {String} host The tracker, as the sidebar groups it
     */
    show: function (host) {
        Deluge.TrackerSettingsWindow.superclass.show.call(this);
        this.host = host;
        this.setTitle(String.format(_('Tracker Settings: {0}'), host));

        // Blank while the configuration is on its way, rather than the
        // previous tracker's rules sitting there looking like this one's.
        this.torrents = null;
        this.setOptions({});
        this.setPreview(_('Looking at what is here...'));
        this.load();
        this.loadTorrents();
    },

    /**
     * Reads the rules for this tracker out of the daemon's configuration.
     */
    load: function () {
        deluge.client.core.get_config({
            success: function (config) {
                if (!this.isVisible()) return;
                var trackers = this.trackersOf(config);
                this.setOptions(trackers[this.host] || {});
            },
            failure: function () {
                // The daemon may not be connected. An empty form beats an
                // error dialog over a window somebody just opened.
                if (!this.isVisible()) return;
                this.setOptions({});
            },
            scope: this,
        });
    },

    /**
     * The torrents this tracker has, so the window can say what it would do.
     *
     * One call, when the window opens. The rules are about torrents that have
     * finished, and nothing finishes while somebody fills in a form.
     */
    loadTorrents: function () {
        var host = this.host;
        deluge.client.core.get_torrents_status(
            { tracker_host: host },
            [
                'name',
                'is_finished',
                'completed_time',
                'time_added',
                'total_wanted',
                'save_path',
                'label',
            ],
            {
                success: function (torrents) {
                    if (!this.isVisible() || this.host !== host) return;
                    this.torrents = [];
                    for (var id in torrents || {}) {
                        this.torrents.push(torrents[id]);
                    }
                    this.describe();
                },
                failure: function () {
                    if (!this.isVisible() || this.host !== host) return;
                    this.torrents = null;
                    this.setPreview('');
                },
                scope: this,
            }
        );
    },

    setPreview: function (html) {
        if (this.preview && this.preview.rendered) {
            this.preview.setText(html, false);
        } else if (this.preview) {
            this.preview.text = html;
        }
    },

    /**
     * When a torrent counts as having finished, for a given kind of rule.
     *
     * This mirrors `tracker::finished_at` in the daemon, including its
     * asymmetry: the destructive rule will not work from a completion time
     * libtorrent never recorded, and the others fall back to when the torrent
     * was added. Changing one without the other makes this window lie, which
     * is worse than it saying nothing.
     */
    finishedAt: function (torrent, destructive) {
        var completed = Number(torrent['completed_time']) || 0;
        if (completed > 0) return completed;
        if (destructive) return 0;
        return Number(torrent['time_added']) || 0;
    },

    /**
     * Says, in a sentence, what the form in front of you would do.
     *
     * An estimate and phrased as one: it is the same arithmetic the daemon
     * does, on the same numbers, but the daemon is the one that acts.
     */
    describe: function () {
        if (!this.torrents) return;
        if (!this.torrents.length) {
            this.setPreview(
                Ext.util.Format.htmlEncode(
                    _('No torrents announce to this tracker right now.')
                )
            );
            return;
        }

        var options = this.options();
        var now = new Date().getTime() / 1000;
        var finished = [];
        Ext.each(this.torrents, function (torrent) {
            if (torrent['is_finished']) finished.push(torrent);
        });

        var lines = [
            String.format(
                _('{0} torrents here, {1} of them finished.'),
                this.torrents.length,
                finished.length
            ),
        ];

        if (options['auto_remove']) {
            lines.push(this.describeRemoval(finished, options, now));
        }
        if (options['auto_move'] && options['move_path']) {
            lines.push(this.describeMove(finished, options, now));
        }
        if (options['auto_label'] && options['label']) {
            lines.push(this.describeLabel(finished, options));
        }
        if (options['auto_limit']) {
            lines.push(
                String.format(
                    _('All {0} would be held to these limits.'),
                    this.torrents.length
                )
            );
        }

        this.setPreview(lines.join('<br/>'));
    },

    describeRemoval: function (finished, options, now) {
        var due = [];
        var freed = 0;
        Ext.each(
            finished,
            function (torrent) {
                var from = this.finishedAt(torrent, true);
                if (from <= 0) return;
                due.push(from + options['remove_after_hours'] * 3600);
                freed += Number(torrent['total_wanted']) || 0;
            },
            this
        );

        if (!due.length) {
            return _(
                'Nothing would be removed: none of the finished ones has a completion time the daemon saw.'
            );
        }
        due.sort(function (a, b) {
            return a - b;
        });

        var fate = options['remove_data']
            ? String.format(
                  _('{0} would be removed with their files, freeing {1}'),
                  due.length,
                  fsize(freed)
              )
            : String.format(
                  _('{0} would be removed, their files kept'),
                  due.length
              );
        return fate + ', ' + this.when(due[0], now) + '.';
    },

    describeMove: function (finished, options, now) {
        var due = [];
        var destination = String(options['move_path']).replace(/\/+$/, '');
        Ext.each(
            finished,
            function (torrent) {
                var where = String(torrent['save_path'] || '').replace(
                    /\/+$/,
                    ''
                );
                if (where === destination) return;
                var from = this.finishedAt(torrent, false);
                if (from <= 0) return;
                due.push(from + options['move_after_hours'] * 3600);
            },
            this
        );

        if (!due.length) {
            return _('Nothing would be moved: they are already there.');
        }
        due.sort(function (a, b) {
            return a - b;
        });
        return (
            String.format(
                _('{0} would be moved to {1}'),
                due.length,
                Ext.util.Format.htmlEncode(options['move_path'])
            ) +
            ', ' +
            this.when(due[0], now) +
            '.'
        );
    },

    describeLabel: function (finished, options) {
        var label = String(options['label']).trim().toLowerCase();
        var onArrival = 0;
        var onCompletion = 0;
        Ext.each(
            this.torrents,
            function (torrent) {
                var current = String(torrent['label'] || '');
                if (current === label) return;
                if (options['label_on_add'] && !current) onArrival++;
                else if (options['label_when_done'] && torrent['is_finished'])
                    onCompletion++;
            },
            this
        );

        var total = onArrival + onCompletion;
        if (!total) {
            return String.format(
                _('Every torrent here is already in {0}.'),
                Ext.util.Format.htmlEncode(options['label'])
            );
        }
        return String.format(
            _('{0} would be put in {1}.'),
            total,
            Ext.util.Format.htmlEncode(options['label'])
        );
    },

    /**
     * "in 21h", or "at the next sweep" for something already due.
     */
    when: function (at, now) {
        var left = Math.round(at - now);
        if (left <= 0) return _('at the next sweep, about a minute away');
        return String.format(_('the first in {0}'), ftime(left));
    },

    /**
     * The `tracker` key's register, whatever shape the configuration is in.
     */
    trackersOf: function (config) {
        var tracker = (config && config['tracker']) || {};
        return tracker['trackers'] || {};
    },

    /**
     * Fills the window in from one tracker's entry.
     *
     * A setting the entry does not carry is the default, not a blank: an entry
     * written by an older build, or by hand, has only the keys somebody set.
     */
    setOptions: function (options) {
        for (var name in this.fields) {
            var field = this.fields[name];
            var value = options[name];
            var fallback = this.defaults[name];
            if (value === undefined || value === null) value = fallback;

            if (field.getXType() === 'checkbox') {
                field.setValue(value === true);
            } else if (field.getXType() === 'spinnerfield') {
                field.setValue(Deluge.number(value, fallback));
            } else {
                field.setValue(value);
            }
        }
        this.onSwitched();
    },

    /**
     * A rule's fields mean nothing until its switch is on, so they follow it.
     */
    onSwitched: function () {
        Ext.each(
            this.groups,
            function (group) {
                var on = this.fields[group.key].getValue() === true;
                Ext.each(
                    group.fields,
                    function (field) {
                        // A field can depend on another one inside its group:
                        // a delay after the torrent finishes means nothing
                        // until somebody asks for anything to happen then.
                        var wanted =
                            on &&
                            (!field.needs ||
                                this.fields[field.needs].getValue() === true);
                        this.fields[field.name].setDisabled(!wanted);
                    },
                    this
                );
                // A warning that is always there is one nobody reads by the
                // third time they open this window.
                Ext.each(
                    group.warnings,
                    function (warning) {
                        var field = this.fields[warning.field];
                        warning.label.setVisible(on && field.getValue() === true);
                    },
                    this
                );
            },
            this
        );
        this.describe();
    },

    /**
     * What the window is asking for, as the configuration stores it.
     */
    options: function () {
        var options = {};
        for (var name in this.fields) {
            var field = this.fields[name];
            if (field.getXType() === 'checkbox') {
                options[name] = field.getValue() === true;
            } else if (field.getXType() === 'spinnerfield') {
                // A blank number field reads as NaN and serialises as null,
                // which the daemon has had to defend against once already.
                options[name] = Deluge.number(
                    field.getValue(),
                    this.defaults[name]
                );
            } else {
                options[name] = field.getValue() || '';
            }
        }
        return options;
    },

    onCancel: function () {
        this.hide();
    },

    /**
     * Writes the rules back.
     *
     * Read, merge, write rather than write: `core.set_config` replaces the
     * whole `tracker` dictionary, so sending only this tracker's entry would
     * quietly delete every other tracker's rules. The configuration is read
     * again here rather than reusing what the window loaded, because the
     * window may have been open for a while.
     */
    onOk: function () {
        var host = this.host;
        if (!host) {
            this.hide();
            return;
        }
        var options = this.options();

        deluge.client.core.get_config({
            success: function (config) {
                var trackers = Ext.apply({}, this.trackersOf(config));
                // Merged over whatever the entry already had, so a setting
                // this build does not know about is not dropped by opening
                // the window and pressing OK.
                trackers[host] = Ext.apply(
                    Ext.apply({}, trackers[host] || {}),
                    options
                );
                deluge.client.core.set_config(
                    { tracker: { trackers: trackers } },
                    {
                        failure: function () {
                            this.complain(
                                _('The daemon did not take the change.')
                            );
                        },
                        scope: this,
                    }
                );
                this.hide();
            },
            failure: function () {
                this.complain(
                    _('The daemon did not answer; nothing was changed.')
                );
            },
            scope: this,
        });
    },

    complain: function (message) {
        Ext.MessageBox.show({
            title: _('Tracker Settings'),
            msg: message,
            buttons: Ext.MessageBox.OK,
            icon: Ext.MessageBox.ERROR,
        });
    },
});

/**
 * The rules a tracker can carry.
 *
 * Each entry is one fieldset in the window and one group of keys under the
 * tracker's entry in the configuration:
 *
 *   key      the setting the switch writes, and the name of the group
 *   title    the fieldset's heading
 *   boxLabel what the switch itself says
 *   note     a sentence under the group, or nothing
 *   fields   what the switch governs, each `{name, ...}` plus any Ext config
 *            `warn` on a field is a line shown only while that field is on
 */
Deluge.TrackerSettingsWindow.RULES = [
    {
        key: 'auto_limit',
        title: _('Limit these torrents'),
        boxLabel: _('Hold them to these limits'),
        note: _(
            'Applied when a torrent from this tracker is first seen, and again whenever you change these numbers. A limit you set on one torrent by hand stands until then: the rule does not fight you every minute. -1 is no limit.'
        ),
        fields: [
            {
                name: 'max_download_speed',
                xtype: 'spinnerfield',
                fieldLabel: _('Maximum download (KiB/s):'),
                labelSeparator: '',
                width: 80,
                decimalPrecision: 1,
                minValue: -1,
                maxValue: 9999999,
                incrementValue: 1,
                value: -1,
            },
            {
                name: 'max_upload_speed',
                xtype: 'spinnerfield',
                fieldLabel: _('Maximum upload (KiB/s):'),
                labelSeparator: '',
                width: 80,
                decimalPrecision: 1,
                minValue: -1,
                maxValue: 9999999,
                incrementValue: 1,
                value: -1,
            },
            {
                name: 'max_connections',
                xtype: 'spinnerfield',
                fieldLabel: _('Maximum connections:'),
                labelSeparator: '',
                width: 80,
                decimalPrecision: 0,
                minValue: -1,
                maxValue: 9999999,
                incrementValue: 1,
                value: -1,
            },
            {
                name: 'max_upload_slots',
                xtype: 'spinnerfield',
                fieldLabel: _('Maximum upload slots:'),
                labelSeparator: '',
                width: 80,
                decimalPrecision: 0,
                minValue: -1,
                maxValue: 9999999,
                incrementValue: 1,
                value: -1,
            },
        ],
    },
    {
        key: 'auto_label',
        title: _('Label these torrents'),
        boxLabel: _('Put them in a label'),
        note: _(
            'The label is created if it does not exist yet, and whatever that label applies is applied, exactly as if the torrent had been put in it by hand.'
        ),
        fields: [
            {
                name: 'label',
                xtype: 'textfield',
                fieldLabel: _('Label:'),
                labelSeparator: '',
                width: 180,
            },
            {
                name: 'label_on_add',
                xtype: 'checkbox',
                hideLabel: true,
                boxLabel: _('When the torrent arrives, if it has no label yet'),
                // The daemon's default for this one is on: a rule that names
                // a label and says nothing about when means the obvious thing.
                dflt: true,
            },
            {
                name: 'label_when_done',
                xtype: 'checkbox',
                hideLabel: true,
                boxLabel: _('When it has finished, replacing whatever it has'),
            },
            {
                name: 'label_after_hours',
                xtype: 'spinnerfield',
                needs: 'label_when_done',
                fieldLabel: _('Hours to wait then:'),
                labelSeparator: '',
                width: 80,
                decimalPrecision: 1,
                minValue: 0,
                maxValue: 87600,
                incrementValue: 1,
                value: 0,
            },
        ],
    },
    {
        key: 'auto_move',
        title: _('Move finished torrents'),
        boxLabel: _('Move their files somewhere else'),
        note: _(
            'Checked against the destination disk first, which is not always the one the files are on now: a folder inside the download folder can be another drive. If the move would leave under a gibibyte free there, it is not started and the log says so. A move within one disk costs nothing and is never refused.'
        ),
        fields: [
            {
                name: 'move_path',
                xtype: 'textfield',
                fieldLabel: _('Move to:'),
                labelSeparator: '',
                width: 240,
            },
            {
                name: 'move_after_hours',
                xtype: 'spinnerfield',
                fieldLabel: _('Hours to wait:'),
                labelSeparator: '',
                width: 80,
                decimalPrecision: 1,
                minValue: 0,
                maxValue: 87600,
                incrementValue: 1,
                value: 0,
            },
        ],
    },
    {
        key: 'auto_remove',
        title: _('Remove finished torrents'),
        boxLabel: _('Remove them after a while'),
        note: _(
            'The wait is measured from the moment the download finished, so a tracker that asks for a day of seeding can be given a day and then clean up after itself. A torrent whose completion time the daemon never saw is left alone.'
        ),
        fields: [
            {
                name: 'remove_after_hours',
                xtype: 'spinnerfield',
                fieldLabel: _('Hours to wait:'),
                labelSeparator: '',
                width: 80,
                decimalPrecision: 1,
                minValue: 0,
                maxValue: 87600,
                incrementValue: 1,
                value: 0,
            },
            {
                name: 'remove_data',
                xtype: 'checkbox',
                hideLabel: true,
                boxLabel: _('Delete the downloaded files as well'),
                warn: _(
                    'The files will be deleted from disk. There is no undo, and nothing else is asked first.'
                ),
            },
        ],
    },
];
