/**
 * Deluge.add.OptionsPanel.js
 *
 * Copyright (c) Damien Churchill 2009-2010 <damoxc@gmail.com>
 *
 * This file is part of Deluge and is licensed under GNU General Public License 3.0, or later, with
 * the additional special exception to link portions of this program with the OpenSSL library.
 * See LICENSE for more details.
 */
Ext.ns('Deluge.add');

/**
 * @class Deluge.add.OptionsTab
 * @extends Ext.form.FormPanel
 */
Deluge.add.OptionsTab = Ext.extend(Ext.form.FormPanel, {
    title: _('Options'),
    height: 170,
    border: false,
    bodyStyle: 'padding: 5px',
    disabled: true,
    labelWidth: 1,
    // The tab is taller than the window gives it, so without this the last
    // fieldset was simply cut off and there was no way to reach it.
    autoScroll: true,

    initComponent: function () {
        Deluge.add.OptionsTab.superclass.initComponent.call(this);

        this.optionsManager = new Deluge.MultiOptionsManager();

        var fieldset = this.add({
            xtype: 'fieldset',
            title: _('Download Folder'),
            border: false,
            autoHeight: true,
            defaultType: 'textfield',
            labelWidth: 1,
            fieldLabel: '',
            style: 'padding: 5px 0; margin-bottom: 0;',
        });
        var location = fieldset.add({
            fieldLabel: '',
            name: 'download_location',
            anchor: '95%',
            labelSeparator: '',
            listeners: {
                change: { fn: this.checkFreeSpace, scope: this },
                blur: { fn: this.checkFreeSpace, scope: this },
            },
        });
        this.optionsManager.bind('download_location', location);
        // Kept because the options manager has no way to hand a field back,
        // and this is the only one anything here has to read directly.
        this.location = location;

        // The disk-space rule catches a full disk after the fact, by pausing
        // everything that is writing. This is the same fact said before
        // anything is written, which is the only moment it is cheap to act on.
        this.space = fieldset.add({
            xtype: 'label',
            text: '',
            style: 'display: block; margin: 2px 0 0 2px; opacity: 0.72;',
        });
        var fieldset = this.add({
            xtype: 'fieldset',
            title: _('Move Completed Folder'),
            border: false,
            autoHeight: true,
            defaultType: 'togglefield',
            labelWidth: 1,
            fieldLabel: '',
            style: 'padding: 5px 0; margin-bottom: 0;',
        });
        var field = fieldset.add({
            fieldLabel: '',
            name: 'move_completed_path',
            anchor: '98%',
        });
        this.optionsManager.bind('move_completed', field.toggle);
        this.optionsManager.bind('move_completed_path', field.input);

        // A label is a torrent option like any other here, which is why there
        // is no page for it in Preferences: the daemon stores it with the
        // torrent and the sidebar counts it. Until this field existed nothing
        // in the interface could set one, so the sidebar's Labels list was
        // always empty however many torrents there were.
        fieldset = this.add({
            xtype: 'fieldset',
            title: _('Label'),
            border: false,
            autoHeight: true,
            defaultType: 'textfield',
            labelWidth: 1,
            fieldLabel: '',
            style: 'padding: 5px 0; margin-bottom: 0;',
        });
        this.optionsManager.bind(
            'label',
            fieldset.add({
                fieldLabel: '',
                name: 'label',
                anchor: '95%',
                labelSeparator: '',
            })
        );

        var panel = this.add({
            border: false,
            layout: 'column',
            defaultType: 'fieldset',
        });

        fieldset = panel.add({
            title: _('Bandwidth'),
            border: false,
            autoHeight: true,
            bodyStyle: 'padding: 2px 5px',
            labelWidth: 105,
            width: 200,
            defaultType: 'spinnerfield',
            style: 'padding-right: 10px;',
        });
        this.optionsManager.bind(
            'max_download_speed',
            fieldset.add({
                fieldLabel: _('Max Down Speed'),
                name: 'max_download_speed',
                width: 60,
            })
        );
        this.optionsManager.bind(
            'max_upload_speed',
            fieldset.add({
                fieldLabel: _('Max Up Speed'),
                name: 'max_upload_speed',
                width: 60,
            })
        );
        this.optionsManager.bind(
            'max_connections',
            fieldset.add({
                fieldLabel: _('Max Connections'),
                name: 'max_connections',
                width: 60,
            })
        );
        this.optionsManager.bind(
            'max_upload_slots',
            fieldset.add({
                fieldLabel: _('Max Upload Slots'),
                name: 'max_upload_slots',
                width: 60,
            })
        );

        fieldset = panel.add({
            // title: _('General'),
            border: false,
            autoHeight: true,
            defaultType: 'checkbox',
        });
        this.optionsManager.bind(
            'add_paused',
            fieldset.add({
                name: 'add_paused',
                boxLabel: _('Add In Paused State'),
                fieldLabel: '',
                labelSeparator: '',
            })
        );
        this.optionsManager.bind(
            'prioritize_first_last_pieces',
            fieldset.add({
                name: 'prioritize_first_last_pieces',
                boxLabel: _('Prioritize First/Last Pieces'),
                fieldLabel: '',
                labelSeparator: '',
            })
        );
        this.optionsManager.bind(
            'sequential_download',
            fieldset.add({
                name: 'sequential_download',
                boxLabel: _('Sequential Download'),
                fieldLabel: '',
                labelSeparator: '',
            })
        );
        this.optionsManager.bind(
            'seed_mode',
            fieldset.add({
                name: 'seed_mode',
                boxLabel: _('Skip File Hash Check'),
                fieldLabel: '',
                labelSeparator: '',
            })
        );
        this.optionsManager.bind(
            'super_seeding',
            fieldset.add({
                name: 'super_seeding',
                boxLabel: _('Super Seed'),
                fieldLabel: '',
                labelSeparator: '',
            })
        );
        this.optionsManager.bind(
            'pre_allocate_storage',
            fieldset.add({
                name: 'pre_allocate_storage',
                boxLabel: _('Preallocate Disk Space'),
                fieldLabel: '',
                labelSeparator: '',
            })
        );
    },

    /**
     * Told what the selected torrent will take, when the selection changes.
     */
    setTorrentSize: function (bytes) {
        this.torrentSize = Number(bytes) || 0;
        this.checkFreeSpace();
    },

    /**
     * Asks the daemon what is free where this torrent is about to be written.
     *
     * The daemon answers for the filesystem the path is on, which is not
     * always the one the path looks like it is on: a folder inside the
     * download folder can be a mount point for another disk.
     */
    checkFreeSpace: function () {
        var path = this.location ? this.location.getValue() : '';
        if (!path) {
            this.setSpaceText('', false);
            return;
        }
        // The answer is about a path, and the path can change while the call
        // is out; anything that comes back about another one is dropped.
        this.pending = path;

        deluge.client.core.get_free_space(path, {
            success: function (free) {
                if (this.pending !== path) return;
                this.showFreeSpace(path, Number(free));
            },
            failure: function () {
                if (this.pending !== path) return;
                this.setSpaceText('', false);
            },
            scope: this,
        });
    },

    showFreeSpace: function (path, free) {
        if (!(free >= 0)) {
            // -1 is the daemon saying it could not look, which happens for a
            // path that does not exist yet and is not worth alarming anybody
            // about: the daemon creates it.
            this.setSpaceText('', false);
            return;
        }

        var size = this.torrentSize || 0;
        if (!size) {
            this.setSpaceText(
                String.format(_('{0} free at {1}'), fsize(free), path),
                false
            );
            return;
        }

        if (free >= size) {
            this.setSpaceText(
                String.format(
                    _('{0} needed, {1} free at {2}'),
                    fsize(size),
                    fsize(free),
                    path
                ),
                false
            );
            return;
        }
        this.setSpaceText(
            String.format(
                _('{0} needed, only {1} free at {2}'),
                fsize(size),
                fsize(free),
                path
            ),
            true
        );
    },

    setSpaceText: function (text, short) {
        if (!this.space) return;
        this.space.setText(text);
        if (!this.space.el) return;
        // Said in red rather than refused: the torrent may be one the person
        // means to add and make room for, and a client that argues with you
        // about your own disk is worse than one that tells you.
        this.space.el.setStyle('color', short ? '#a03030' : '#666');
    },

    getDefaults: function () {
        var keys = [
            'add_paused',
            'pre_allocate_storage',
            'download_location',
            'max_connections_per_torrent',
            'max_download_speed_per_torrent',
            'move_completed',
            'move_completed_path',
            'max_upload_slots_per_torrent',
            'max_upload_speed_per_torrent',
            'prioritize_first_last_pieces',
            'sequential_download',
        ];

        deluge.client.core.get_config_values(keys, {
            success: function (config) {
                var options = {
                    file_priorities: [],
                    add_paused: config.add_paused,
                    sequential_download: config.sequential_download,
                    pre_allocate_storage: config.pre_allocate_storage,
                    download_location: config.download_location,
                    move_completed: config.move_completed,
                    move_completed_path: config.move_completed_path,
                    max_connections: config.max_connections_per_torrent,
                    max_download_speed: config.max_download_speed_per_torrent,
                    max_upload_slots: config.max_upload_slots_per_torrent,
                    max_upload_speed: config.max_upload_speed_per_torrent,
                    prioritize_first_last_pieces:
                        config.prioritize_first_last_pieces,
                    seed_mode: false,
                    super_seeding: false,
                    // No configured default: a label is per torrent, and a
                    // global one would put every torrent in the same bucket.
                    label: '',
                };
                this.optionsManager.options = options;
                this.optionsManager.resetAll();
            },
            scope: this,
        });
    },
});
