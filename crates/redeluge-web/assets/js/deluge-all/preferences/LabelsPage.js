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
 * What each label *applies* is not here. It was, under the grid, filled in
 * when you selected a row — which made selecting a label an edit rather than a
 * look, and left an Apply armed with whichever row had been touched last. It
 * lives in `Deluge.LabelSettingsWindow` now, one window per label, reached
 * from the Edit button here and from the label's own row in the sidebar.
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
                'A label groups torrents. Put a torrent in one from its Options tab, or let another program do it: this is the Label plugin’s own list, so anything that speaks Deluge’s API sees the same labels. Pick one and press Edit for what it applies to the torrents in it.'
            ),
            style: 'display: block; margin-bottom: 6px; opacity: 0.72;',
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
                    text: _('Edit'),
                    iconCls: 'x-deluge-preferences',
                    handler: this.onEditLabel,
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
        // Double-clicking a row opens it, which is what double-clicking a row
        // means everywhere else. Renaming has a button of its own.
        this.grid.on('rowdblclick', this.onEditLabel, this);
        this.grid.getSelectionModel().on('selectionchange', this.onSelect, this);

        this.on('show', this.onPageShow, this);
    },

    // The card layout fires `show` on every switch back; reading once is
    // enough, and every button here reloads after it has changed something.
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

        // A reload that dropped the selection would move the buttons' target
        // out from under whoever is using them.
        if (keep) {
            var index = this.store.find('label', keep);
            if (index > -1) {
                this.grid.getSelectionModel().selectRow(index);
            }
        }
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
        if (options.apply_stuck) parts.push(_('removes stuck downloads'));
        if (options.hide_by_default) parts.push(_('hidden by default'));
        return parts.length ? parts.join(', ') : _('nothing');
    },

    selected: function () {
        var record = this.grid.getSelectionModel().getSelected();
        return record ? record.get('label') : null;
    },

    /**
     * Selecting a row now only says which row the buttons act on. It used to
     * fill in a form, which meant you could not read what a label applied
     * without arming an Apply that would write it back.
     */
    onSelect: function () {
        this.keepSelected = this.selected();
    },

    /**
     * Opens the selected label's settings, in the window the sidebar opens too.
     */
    onEditLabel: function () {
        var name = this.selected();
        if (!name) return;
        var window = Deluge.LabelSettingsWindow.open(name);
        // The grid's last column says what each label applies, so it is out of
        // date the moment the window writes. Listened to once per opening.
        window.un('saved', this.reload, this);
        window.on('saved', this.reload, this, { single: true });
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
});
