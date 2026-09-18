/**
 * Deluge.LabelSettingsWindow.js
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 *
 * What a label does to the torrents in it.
 *
 * These options used to sit under the list in Preferences, filled in when you
 * selected a row, which made the list a control rather than a list: selecting
 * a label to read what it applied also armed an Apply that would write it
 * back, and the form under the grid belonged to whichever row had been touched
 * last. A window belongs to one label, says so in its title, and is closed by
 * Cancel.
 *
 * It is reached from the two places a label is: its row in the sidebar, and
 * its row in Preferences. The tracker rules window is the same shape for the
 * same reason, so the two behave alike.
 *
 * Everything here goes through `label.set_options`, which merges what it is
 * sent over what is stored, so this window sends its own fields and nothing
 * else — an option a future version adds is not dropped by opening this one
 * and pressing OK.
 */
Ext.ns('Deluge');

/**
 * @class Deluge.LabelSettingsWindow
 * @extends Ext.Window
 */
Deluge.LabelSettingsWindow = Ext.extend(Ext.Window, {
    title: _('Label Settings'),
    width: 470,
    // Tall enough for the four boxes without scrolling, which is the whole
    // point of separating them: a group you have to scroll to find is a group
    // you did not know was there.
    height: 540,
    layout: 'fit',
    buttonAlign: 'right',
    closeAction: 'hide',
    constrainHeader: true,
    plain: true,
    minWidth: 360,
    minHeight: 280,

    initComponent: function () {
        Deluge.LabelSettingsWindow.superclass.initComponent.call(this);

        // Whoever opened this window may be showing the same options in a
        // list, so it says when it has written something.
        this.addEvents('saved');

        this.addButton(_('Cancel'), this.onCancel, this);
        this.addButton(_('OK'), this.onOk, this);

        this.form = this.add({
            xtype: 'form',
            border: false,
            autoScroll: true,
            bodyStyle: 'padding: 5px',
            labelWidth: 170,
        });

        this.fields = {};

        // A box per group rather than three switches in one column. Which four
        // fields a switch governs was a matter of counting indents before, and
        // the window is mostly fields.
        var bandwidth = this.group('apply_max', _('Bandwidth limits'));
        this.fields.max_download_speed = bandwidth.add(
            this.spinner(_('Maximum download (KiB/s):'), 1)
        );
        this.fields.max_upload_speed = bandwidth.add(
            this.spinner(_('Maximum upload (KiB/s):'), 1)
        );
        this.fields.max_connections = bandwidth.add(
            this.spinner(_('Maximum connections:'), 0)
        );
        this.fields.max_upload_slots = bandwidth.add(
            this.spinner(_('Maximum upload slots:'), 0)
        );

        var seeding = this.group('apply_queue', _('Seeding rules'));
        this.fields.stop_at_ratio = seeding.add({
            xtype: 'checkbox',
            hideLabel: true,
            boxLabel: _('Stop seeding at ratio'),
        });
        this.fields.stop_ratio = seeding.add(this.spinner(_('Ratio:'), 1, 0.1));
        this.fields.remove_at_ratio = seeding.add({
            xtype: 'checkbox',
            hideLabel: true,
            boxLabel: _('Remove the torrent at that ratio'),
        });

        var move = this.group('apply_move_completed', _('Move on completion'));
        this.fields.move_completed_path = move.add({
            xtype: 'textfield',
            fieldLabel: _('Move to:'),
            labelSeparator: '',
            width: 220,
        });

        // The fourth box is not a group: it has no switch, because it is the
        // one option here that does nothing to the torrents. It decides what
        // the list shows, so it says so in a legend rather than in a switch.
        var view = this.form.add({
            xtype: 'fieldset',
            cls: 'x-deluge-option-group',
            title: _('In the torrent list'),
            autoHeight: true,
            labelWidth: 170,
        });
        this.fields.hide_by_default = view.add({
            xtype: 'checkbox',
            hideLabel: true,
            boxLabel: _('Hide these torrents unless asked for'),
        });
        view.add({
            xtype: 'label',
            text: _(
                'They are still there: pick the label in the sidebar to see them, or tick it under Show labels in the Label column’s header menu.'
            ),
            style: 'display: block; margin: 2px 0 0 0; color: #666;',
        });
    },

    /**
     * One box: the switch that governs a group, and the fields it governs.
     *
     * The switch is the box's heading rather than a line inside it, which is
     * what the borrowed indent was trying to say. A group whose switch is off
     * is not a group of zeroes, it is a group this label does not touch, and
     * `onSwitched` greys it whole.
     */
    group: function (key, caption) {
        var set = this.form.add({
            xtype: 'fieldset',
            cls: 'x-deluge-option-group',
            autoHeight: true,
            labelWidth: 170,
        });
        this.fields[key] = set.add({
            xtype: 'checkbox',
            hideLabel: true,
            boxLabel: caption,
            ctCls: 'x-deluge-group-switch',
            handler: this.onSwitched,
            scope: this,
        });
        return set;
    },

    /**
     * A number field, since this window needs six of them.
     */
    spinner: function (caption, precision, increment) {
        return {
            xtype: 'spinnerfield',
            fieldLabel: caption,
            labelSeparator: '',
            width: 80,
            decimalPrecision: precision,
            minValue: -1,
            maxValue: 9999999,
            incrementValue: increment || 1,
        };
    },

    /**
     * Opens the window on one label.
     */
    show: function (name) {
        Deluge.LabelSettingsWindow.superclass.show.call(this);
        this.label = name;
        this.setTitle(String.format(_('Label Settings: {0}'), name));

        // Blank while the options are on their way, rather than the previous
        // label's settings sitting there looking like this one's.
        this.setOptions({});
        this.load();
    },

    load: function () {
        var name = this.label;
        deluge.client.label.get_options(name, {
            success: function (options) {
                if (!this.isVisible() || this.label !== name) return;
                this.setOptions(options || {});
            },
            failure: function () {
                // The label may have been removed, or the daemon may be gone.
                // An empty form beats an error dialog over a window somebody
                // just opened.
                if (!this.isVisible() || this.label !== name) return;
                this.setOptions({});
            },
            scope: this,
        });
    },

    setOptions: function (options) {
        for (var name in this.fields) {
            var field = this.fields[name];
            var value = options[name];

            if (field.getXType() === 'checkbox') {
                field.setValue(value === true);
            } else if (field.getXType() === 'spinnerfield') {
                field.setValue(Deluge.number(value, -1));
            } else {
                field.setValue(value === undefined ? '' : value);
            }
        }
        this.onSwitched();
    },

    /**
     * A group's fields mean nothing until its switch is on, so they follow it.
     */
    onSwitched: function () {
        for (var group in Deluge.LabelSettingsWindow.GROUPS) {
            var on = this.fields[group].getValue() === true;
            Ext.each(
                Deluge.LabelSettingsWindow.GROUPS[group],
                function (name) {
                    this.fields[name].setDisabled(!on);
                },
                this
            );
        }
    },

    options: function () {
        var options = {};
        for (var name in this.fields) {
            var field = this.fields[name];
            if (field.getXType() === 'checkbox') {
                options[name] = field.getValue() === true;
            } else if (field.getXType() === 'spinnerfield') {
                // A blank number field reads as NaN and serialises as null,
                // which the daemon has had to defend against once already.
                options[name] = Deluge.number(field.getValue(), -1);
            } else {
                options[name] = field.getValue() || '';
            }
        }
        // The plugin keeps these two together, and the path is meaningless
        // without the switch that turns moving on.
        options['move_completed'] = options['apply_move_completed'];
        return options;
    },

    onCancel: function () {
        this.hide();
    },

    onOk: function () {
        var name = this.label;
        if (!name) {
            this.hide();
            return;
        }
        var options = this.options();

        // The list is showing whatever was decided when it loaded, so turning
        // hiding on or off here has to reach it now. Without this the checkbox
        // appears to do nothing until the next reload.
        if (deluge.torrents && deluge.torrents.setLabelHidden) {
            deluge.torrents.setLabelHidden(
                name,
                options['hide_by_default'] === true
            );
        }

        deluge.client.label.set_options(name, options, {
            success: function () {
                this.fireEvent('saved', this, name);
                if (deluge.ui) deluge.ui.update();
            },
            failure: function () {
                Ext.MessageBox.show({
                    title: _('Label Settings'),
                    msg: _('The daemon did not take the change.'),
                    buttons: Ext.MessageBox.OK,
                    icon: Ext.MessageBox.WARNING,
                    iconCls: 'x-deluge-icon-warning',
                });
            },
            scope: this,
        });
        this.hide();
    },
});

/**
 * Which fields each switch governs.
 *
 * The same grouping the daemon applies them in: a switch that is off means the
 * label does not touch those settings at all, so the fields under it are not
 * "zero", they are "not this label's business".
 */
Deluge.LabelSettingsWindow.GROUPS = {
    apply_max: [
        'max_download_speed',
        'max_upload_speed',
        'max_connections',
        'max_upload_slots',
    ],
    apply_queue: ['stop_at_ratio', 'stop_ratio', 'remove_at_ratio'],
    apply_move_completed: ['move_completed_path'],
};

/**
 * The one window, built when something first asks for it.
 *
 * Both doors — the sidebar's menu and the Preferences list — open the same
 * window, so a label's settings cannot be open twice saying two things.
 */
Deluge.LabelSettingsWindow.open = function (name) {
    if (!deluge.labelSettings) {
        deluge.labelSettings = new Deluge.LabelSettingsWindow();
    }
    deluge.labelSettings.show(name);
    return deluge.labelSettings;
};
