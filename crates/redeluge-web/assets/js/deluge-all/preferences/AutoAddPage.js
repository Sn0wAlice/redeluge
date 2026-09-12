/**
 * Deluge.preferences.AutoAdd.js
 *
 * Copyright (c) 2026 the redeluge contributors
 *
 * This file is part of redeluge and is licensed under GNU General Public License 3.0, or later,
 * with the additional special exception to link portions of this program with the OpenSSL library.
 * See LICENSE for more details.
 */
Ext.namespace('Deluge.preferences');

/**
 * @class Deluge.preferences.AutoAdd
 * @extends Ext.Panel
 *
 * Watched directories. Was the AutoAdd plugin; now the `autoadd` key of
 * core.conf, one dictionary holding a list of directories.
 *
 * The list is an editable grid rather than a dialog per directory: every field
 * is a short string or a choice, and a grid shows all of them at once, which
 * is what you want when deciding where a new one should go.
 */
Deluge.preferences.AutoAdd = Ext.extend(Ext.Panel, {
    border: false,
    title: _('Watched Folders'),
    header: false,
    layout: 'form',
    autoScroll: true,

    initComponent: function () {
        Deluge.preferences.AutoAdd.superclass.initComponent.call(this);

        // The label column is wide enough for the longest caption on the
        // page. A form layout positions every field at that offset, so a
        // narrower column does not wrap the caption, it draws the field on top
        // of it: "Scan every (seconds):" came out as three lines with the
        // spinner across them. The checkbox hides its label rather than
        // setting an empty one, which is what keeps it against the left edge
        // without narrowing the column for everything else.
        var fieldset = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('Watched Folders'),
            autoHeight: true,
            labelWidth: 140,
            style: 'padding-top: 5px; margin-bottom: 0px;',
        });

        this.enabled = fieldset.add({
            xtype: 'checkbox',
            hideLabel: true,
            boxLabel: _('Add torrents dropped into a watched folder'),
            name: 'autoadd_enabled',
        });

        this.interval = fieldset.add({
            xtype: 'spinnerfield',
            fieldLabel: _('Scan every (seconds):'),
            labelSeparator: '',
            name: 'autoadd_interval',
            width: 80,
            decimalPrecision: 0,
            minValue: 1,
            style: 'margin-top: 6px;',
        });

        this.store = new Ext.data.ArrayStore({
            fields: [
                { name: 'enabled', type: 'bool' },
                { name: 'path', type: 'string' },
                { name: 'download_location', type: 'string' },
                { name: 'label', type: 'string' },
                { name: 'add_paused', type: 'bool' },
                { name: 'after_add', type: 'string' },
                { name: 'rename_extension', type: 'string' },
                { name: 'copy_to', type: 'string' },
            ],
        });

        var afterAdd = new Ext.data.SimpleStore({
            fields: ['value', 'text'],
            data: [
                ['rename', _('Rename it')],
                ['leave', _('Leave it')],
                ['delete', _('Delete it')],
            ],
        });

        this.grid = this.add({
            xtype: 'editorgrid',
            store: this.store,
            height: 220,
            // Tracks the page instead of a fixed 560: a grid wider than the
            // panel does not clip, it draws past the frame, and the columns
            // beyond the edge cannot be reached at all. Anchored, the grid's
            // own view scrolls when the columns need more room than there is.
            anchor: '100%',
            clicksToEdit: 1,
            style: 'margin: 8px 0;',
            selModel: new Ext.grid.RowSelectionModel({ singleSelect: true }),
            columns: [
                {
                    header: _('On'),
                    dataIndex: 'enabled',
                    width: 36,
                    renderer: this.renderTick,
                    editor: { xtype: 'checkbox' },
                },
                {
                    header: _('Folder'),
                    dataIndex: 'path',
                    width: 160,
                    editor: { xtype: 'textfield', allowBlank: false },
                },
                {
                    header: _('Download to'),
                    dataIndex: 'download_location',
                    // The six widths add up to the grid's own width at the
                    // window's size, so there is no horizontal scrollbar for
                    // the last few pixels of the last column.
                    width: 128,
                    editor: { xtype: 'textfield' },
                },
                {
                    header: _('Label'),
                    dataIndex: 'label',
                    width: 80,
                    editor: { xtype: 'textfield' },
                },
                {
                    header: _('Paused'),
                    dataIndex: 'add_paused',
                    width: 56,
                    renderer: this.renderTick,
                    editor: { xtype: 'checkbox' },
                },
                {
                    header: _('Afterwards'),
                    dataIndex: 'after_add',
                    width: 90,
                    editor: new Ext.form.ComboBox({
                        store: afterAdd,
                        displayField: 'text',
                        valueField: 'value',
                        mode: 'local',
                        triggerAction: 'all',
                        editable: false,
                    }),
                    renderer: function (value) {
                        var found = afterAdd.data.find(function (item) {
                            return item.data.value == value;
                        });
                        return found ? found.data.text : value;
                    },
                },
            ],
            bbar: [
                {
                    text: _('Add'),
                    iconCls: 'icon-add',
                    handler: this.onAddFolder,
                    scope: this,
                },
                {
                    text: _('Remove'),
                    iconCls: 'icon-remove',
                    handler: this.onRemoveFolder,
                    scope: this,
                },
            ],
        });

        this.add({
            xtype: 'label',
            text: _(
                'A file is added once its size has stopped changing, so a torrent still being written is left alone. Only .torrent and .magnet files are read.'
            ),
            style: 'display: block; margin: 0 0 8px 0; color: #666;',
        });

        this.on('show', this.onPageShow, this);
    },

    renderTick: function (value) {
        return value ? '&#10003;' : '';
    },

    // Not `onAdd`: Ext.Container calls a method of that name itself every time
    // anything is added to the panel, so naming the button handler `onAdd`
    // overrides the container's own hook and runs this against a store that
    // does not exist yet. `onRemove` is the same trap.
    onAddFolder: function () {
        this.store.add(
            new this.store.recordType({
                enabled: true,
                path: '',
                download_location: '',
                label: '',
                add_paused: false,
                after_add: 'rename',
                rename_extension: '.added',
                copy_to: '',
            })
        );
    },

    onRemoveFolder: function () {
        var selected = this.grid.getSelectionModel().getSelected();
        if (selected) this.store.remove(selected);
    },

    // The card layout fires `show` whenever the window opens on this page and
    // again on every switch back to it. Fetching each time would be a call per
    // click; once is enough, and the Apply button writes rather than reads.
    onPageShow: function () {
        if (this.loaded) return;
        this.loaded = true;
        deluge.client.core.get_config({
            success: this.onGotConfig,
            scope: this,
        });
    },

    onGotConfig: function (config) {
        var settings = config['autoadd'] || {};
        this.enabled.setValue(settings['enabled'] === true);
        this.interval.setValue(Ext.value(settings['interval'], 5));

        var rows = (settings['watchdirs'] || []).map(function (dir) {
            return [
                dir['enabled'] !== false,
                Ext.value(dir['path'], ''),
                Ext.value(dir['download_location'], ''),
                Ext.value(dir['label'], ''),
                dir['add_paused'] === true,
                Ext.value(dir['after_add'], 'rename'),
                Ext.value(dir['rename_extension'], '.added'),
                Ext.value(dir['copy_to'], ''),
            ];
        });
        this.store.loadData(rows);
    },

    onApply: function () {
        // Preferences applies every page, including ones nobody opened.
        // Until this page has read the stored settings its fields hold
        // defaults and its grid is empty, and writing that back would replace
        // what is configured with nothing.
        if (!this.loaded) return;

        var watchdirs = [];
        this.store.each(function (record) {
            var path = (record.get('path') || '').trim();
            // A row with no folder is a row someone added and did not fill in.
            if (!path) return;
            watchdirs.push({
                enabled: record.get('enabled') === true,
                path: path,
                download_location: record.get('download_location') || '',
                label: record.get('label') || '',
                add_paused: record.get('add_paused') === true,
                after_add: record.get('after_add') || 'rename',
                rename_extension: record.get('rename_extension') || '.added',
                copy_to: record.get('copy_to') || '',
            });
        });

        deluge.client.core.set_config({
            autoadd: {
                enabled: this.enabled.getValue(),
                interval: Deluge.number(this.interval.getValue(), 5),
                watchdirs: watchdirs,
            },
        });
    },

    onOk: function () {
        this.onApply();
    },
});
