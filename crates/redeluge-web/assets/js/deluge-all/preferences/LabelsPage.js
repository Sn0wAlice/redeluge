/**
 * Deluge.preferences.LabelsPage.js
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 *
 * The register of labels: what exists, and what each one does to the torrents
 * in it.
 *
 * A label is a torrent option, so which torrent carries which label is set on
 * the torrent, in its own Options tab. This page is the other half: the labels
 * themselves, which have to exist before anything is put in them. That is not
 * a nicety. Radarr and Sonarr ask the daemon for the list, and a label with
 * nothing in it yet is exactly the one they are about to start using.
 *
 * It talks to `label.*`, which is the Label plugin's own API, so everything
 * this page does can also be done by anything else that speaks it.
 */
Ext.namespace('Deluge.preferences');

/**
 * @class Deluge.preferences.Labels
 * @extends Ext.Panel
 */
Deluge.preferences.Labels = Ext.extend(Ext.Panel, {
    border: false,
    title: _('Labels'),
    header: false,
    layout: 'form',

    initComponent: function () {
        Deluge.preferences.Labels.superclass.initComponent.call(this);

        var intro = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('Labels'),
            autoHeight: true,
            labelWidth: 1,
            style: 'padding-top: 5px; margin-bottom: 0px;',
        });
        intro.add({
            xtype: 'label',
            text: _(
                'A label groups torrents. Put a torrent in one from its Options tab, or let another program do it: this is the Label plugin’s own list, so anything that speaks Deluge’s API sees the same labels.'
            ),
            style: 'display: block; margin-bottom: 6px; color: #666;',
        });

        this.store = new Ext.data.ArrayStore({
            idIndex: 0,
            fields: [
                { name: 'label', type: 'string' },
                { name: 'torrents', type: 'int' },
                { name: 'rules', type: 'string' },
            ],
        });

        this.grid = this.add({
            xtype: 'grid',
            store: this.store,
            height: 190,
            anchor: '100%',
            style: 'margin: 4px 0 8px 0;',
            selModel: new Ext.grid.RowSelectionModel({ singleSelect: true }),
            columns: [
                {
                    header: _('Label'),
                    dataIndex: 'label',
                    width: 160,
                },
                {
                    header: _('Torrents'),
                    dataIndex: 'torrents',
                    width: 70,
                },
                {
                    header: _('Applies'),
                    dataIndex: 'rules',
                    width: 200,
                },
            ],
            bbar: [
                {
                    text: _('Add'),
                    iconCls: 'icon-add',
                    handler: this.onAddLabel,
                    scope: this,
                },
                {
                    text: _('Rename'),
                    iconCls: 'icon-edit',
                    handler: this.onRenameLabel,
                    scope: this,
                },
                {
                    text: _('Remove'),
                    iconCls: 'icon-remove',
                    handler: this.onRemoveLabel,
                    scope: this,
                },
            ],
        });
        this.grid.on('rowdblclick', this.onRenameLabel, this);
        this.grid.getSelectionModel().on('selectionchange', this.onSelect, this);

        this.options = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('What this label applies'),
            autoHeight: true,
            labelWidth: 170,
            style: 'padding-top: 5px; margin-bottom: 0px;',
        });

        this.fields = {};

        this.fields.apply_max = this.options.add({
            xtype: 'checkbox',
            hideLabel: true,
            boxLabel: _('Bandwidth limits'),
            handler: this.onGroupToggled,
            scope: this,
        });
        this.fields.max_download_speed = this.options.add(
            this.spinner(_('Maximum download (KiB/s):'), 1)
        );
        this.fields.max_upload_speed = this.options.add(
            this.spinner(_('Maximum upload (KiB/s):'), 1)
        );
        this.fields.max_connections = this.options.add(
            this.spinner(_('Maximum connections:'), 0)
        );
        this.fields.max_upload_slots = this.options.add(
            this.spinner(_('Maximum upload slots:'), 0)
        );

        this.fields.apply_queue = this.options.add({
            xtype: 'checkbox',
            hideLabel: true,
            boxLabel: _('Seeding rules'),
            style: 'margin-top: 6px',
            handler: this.onGroupToggled,
            scope: this,
        });
        this.fields.stop_at_ratio = this.options.add({
            xtype: 'checkbox',
            hideLabel: true,
            boxLabel: _('Stop seeding at ratio'),
            ctCls: 'x-deluge-indent-checkbox',
        });
        this.fields.stop_ratio = this.options.add(
            this.spinner(_('Ratio:'), 1, 0.1)
        );
        this.fields.remove_at_ratio = this.options.add({
            xtype: 'checkbox',
            hideLabel: true,
            boxLabel: _('Remove the torrent at that ratio'),
            ctCls: 'x-deluge-indent-checkbox',
        });

        this.fields.apply_move_completed = this.options.add({
            xtype: 'checkbox',
            hideLabel: true,
            boxLabel: _('Move on completion'),
            style: 'margin-top: 6px',
            handler: this.onGroupToggled,
            scope: this,
        });
        this.fields.move_completed_path = this.options.add({
            xtype: 'textfield',
            fieldLabel: _('Move to:'),
            labelSeparator: '',
            width: 220,
        });

        // Its own fieldset, because it is the one option here that does
        // nothing to the torrents: it decides what the list shows.
        var view = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('In the torrent list'),
            autoHeight: true,
            labelWidth: 170,
            style: 'padding-top: 5px; margin-bottom: 0px;',
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

        this.setOptionsEnabled(false);
        this.on('show', this.onPageShow, this);
    },

    /**
     * A number field, since this page needs six of them.
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

    // The card layout fires `show` on every switch back; reading once is
    // enough, and Apply writes.
    onPageShow: function () {
        if (this.loaded) return;
        this.loaded = true;
        this.reload();
    },

    /**
     * Reads the labels, and how many torrents carry each.
     */
    reload: function () {
        deluge.client.label.get_config({
            success: function (config) {
                this.config = config && config.labels ? config.labels : {};
                this.refreshGrid();
            },
            failure: function () {
                // The daemon may not be connected yet. Nothing to show, and
                // nothing worth an error dialog on a preferences page.
                this.config = {};
                this.refreshGrid();
            },
            scope: this,
        });
    },

    refreshGrid: function () {
        // How many torrents carry each label, from the counts the sidebar
        // already has, so this page adds no call of its own. `getStates`
        // answers an object keyed by label, not a list of pairs.
        var counts = {};
        var panel = deluge.sidebar && deluge.sidebar.getFilter('label');
        var states = panel && panel.getStates ? panel.getStates() : null;
        if (states) {
            for (var key in states) {
                counts[key] = states[key];
            }
        }

        var rows = [];
        for (var name in this.config) {
            rows.push([
                name,
                counts[name] || 0,
                this.describe(this.config[name]),
            ]);
        }
        rows.sort(function (a, b) {
            return a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0;
        });
        var keep = this.keepSelected;
        this.store.loadData(rows);

        // Applying reloads, and losing the selection would leave the options
        // below belonging to nothing a moment after they were edited.
        if (keep) {
            var index = this.store.find('label', keep);
            if (index > -1) {
                this.grid.getSelectionModel().selectRow(index);
                return;
            }
        }
        this.setOptionsEnabled(false);
    },

    /**
     * The one-line summary in the grid's last column.
     */
    describe: function (options) {
        var parts = [];
        if (!options) return _('nothing');
        if (options.apply_max) parts.push(_('bandwidth'));
        if (options.apply_queue) parts.push(_('seeding'));
        if (options.apply_move_completed) parts.push(_('move on completion'));
        if (options.hide_by_default) parts.push(_('hidden by default'));
        return parts.length ? parts.join(', ') : _('nothing');
    },

    selected: function () {
        var record = this.grid.getSelectionModel().getSelected();
        return record ? record.get('label') : null;
    },

    onSelect: function () {
        var name = this.selected();
        this.keepSelected = name;
        if (!name) {
            this.setOptionsEnabled(false);
            return;
        }
        var options = this.config[name] || {};
        for (var key in this.fields) {
            var value = options[key];
            this.fields[key].setValue(value === undefined ? '' : value);
        }
        this.setOptionsEnabled(true);
    },

    setOptionsEnabled: function (on) {
        for (var key in this.fields) {
            this.fields[key].setDisabled(!on);
        }
        if (on) this.onGroupToggled();
    },

    /**
     * A group's fields mean nothing until its switch is on, so they follow it.
     */
    onGroupToggled: function () {
        var groups = {
            apply_max: [
                'max_download_speed',
                'max_upload_speed',
                'max_connections',
                'max_upload_slots',
            ],
            apply_queue: ['stop_at_ratio', 'stop_ratio', 'remove_at_ratio'],
            apply_move_completed: ['move_completed_path'],
        };
        for (var group in groups) {
            var on = this.fields[group].getValue() === true;
            Ext.each(
                groups[group],
                function (name) {
                    this.fields[name].setDisabled(!on);
                },
                this
            );
        }
    },

    onAddLabel: function () {
        Ext.MessageBox.prompt(
            _('Add Label'),
            _('Name:'),
            function (button, text) {
                if (button !== 'ok' || !text) return;
                deluge.client.label.add(text, {
                    success: this.reload,
                    failure: this.reload,
                    scope: this,
                });
            },
            this
        );
    },

    /**
     * Renaming is three calls, not one.
     *
     * The Label plugin has no rename, so this adds the new label, moves every
     * torrent across, and removes the old one. Done in that order: a failure
     * anywhere leaves the torrents in a label that still exists.
     */
    onRenameLabel: function () {
        var from = this.selected();
        if (!from) return;

        Ext.MessageBox.prompt(
            _('Rename Label'),
            String.format(_('New name for {0}:'), from),
            function (button, text) {
                if (button !== 'ok' || !text || text === from) return;
                deluge.client.label.add(text, {
                    success: function () {
                        this.moveTorrents(from, text);
                    },
                    failure: function () {
                        this.moveTorrents(from, text);
                    },
                    scope: this,
                });
            },
            this
        );
    },

    moveTorrents: function (from, to) {
        deluge.client.core.get_torrents_status(
            { label: from },
            ['name'],
            {
                success: function (torrents) {
                    var ids = [];
                    for (var id in torrents || {}) {
                        ids.push(id);
                    }
                    if (!ids.length) {
                        this.finishRename(from);
                        return;
                    }
                    deluge.client.core.set_torrent_options(
                        ids,
                        { label: to },
                        {
                            success: function () {
                                this.finishRename(from);
                            },
                            scope: this,
                        }
                    );
                },
                scope: this,
            }
        );
    },

    finishRename: function (from) {
        this.keepSelected = null;
        deluge.client.label.remove(from, {
            success: this.reload,
            failure: this.reload,
            scope: this,
        });
    },

    onRemoveLabel: function () {
        var name = this.selected();
        if (!name) return;

        Ext.MessageBox.confirm(
            _('Remove Label'),
            String.format(
                _(
                    'Remove the label {0}? The torrents in it are kept and lose the label.'
                ),
                name
            ),
            function (button) {
                if (button !== 'yes') return;
                this.keepSelected = null;
                deluge.client.label.remove(name, {
                    success: this.reload,
                    failure: this.reload,
                    scope: this,
                });
            },
            this
        );
    },

    onApply: function () {
        // Nothing has been read yet, so there is nothing of this page's to
        // write; see the other feature pages for why that matters.
        if (!this.loaded) return;
        var name = this.selected();
        if (!name) return;

        var options = {};
        for (var key in this.fields) {
            var field = this.fields[key];
            var value = field.getValue();
            if (field.getXType() === 'checkbox') {
                options[key] = value === true;
            } else if (field.getXType() === 'spinnerfield') {
                options[key] = Deluge.number(value, -1);
            } else {
                options[key] = value || '';
            }
        }
        // The plugin keeps these two together, and the path is meaningless
        // without the switch that turns moving on.
        options['move_completed'] = options['apply_move_completed'];

        // The list is showing whatever was decided when the page loaded, so
        // turning hiding on or off here has to reach it now. Without this the
        // checkbox appears to do nothing until the next reload.
        if (deluge.torrents && deluge.torrents.setLabelHidden) {
            deluge.torrents.setLabelHidden(
                name,
                options['hide_by_default'] === true
            );
        }

        deluge.client.label.set_options(name, options, {
            success: this.reload,
            failure: this.reload,
            scope: this,
        });
    },

    onOk: function () {
        this.onApply();
    },
});
