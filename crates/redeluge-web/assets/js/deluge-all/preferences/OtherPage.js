/**
 * Deluge.preferences.OtherPage.js
 *
 * Copyright (c) Damien Churchill 2009-2010 <damoxc@gmail.com>
 *
 * This file is part of Deluge and is licensed under GNU General Public License 3.0, or later, with
 * the additional special exception to link portions of this program with the OpenSSL library.
 * See LICENSE for more details.
 */
Ext.namespace('Deluge.preferences');

/**
 * @class Deluge.preferences.Other
 * @extends Ext.form.FormPanel
 */
Deluge.preferences.Other = Ext.extend(Ext.form.FormPanel, {
    constructor: function (config) {
        config = Ext.apply(
            {
                border: false,
                title: _('Other'),
                header: false,
                layout: 'form',
            },
            config
        );
        Deluge.preferences.Other.superclass.constructor.call(this, config);
    },

    initComponent: function () {
        Deluge.preferences.Other.superclass.initComponent.call(this);

        var optMan = deluge.preferences.getOptionsManager();

        // What was here: a release check and an anonymous-statistics
        // upload. Neither has anything behind it. redeluge has no update
        // service to ask and sends nothing anywhere, so both were a
        // preference that could be set and could not mean anything.
        var fieldset = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('GeoIP Database'),
            autoHeight: true,
            labelWidth: 80,
            defaultType: 'textfield',
        });
        fieldset.add({
            xtype: 'label',
            text: _(
                'Peers show the flag of their country, which needs a database. Deluge pointed at a system file in a format retired in 2019, so there has been nothing to read for years.'
            ),
            style: 'display: block; margin-bottom: 6px; opacity: 0.72;',
        });
        optMan.bind(
            'geoip_db_location',
            fieldset.add({
                name: 'geoip_db_location',
                fieldLabel: _('Your own file:'),
                labelSeparator: '',
                width: 200,
            })
        );

        // The downloader. Its own dictionary under `countrydb`, so its own
        // read and its own Apply, like the other feature settings.
        this.db = {};
        this.db.enabled = fieldset.add({
            xtype: 'checkbox',
            hideLabel: true,
            boxLabel: _('Or download one and keep it up to date'),
            handler: this.onDatabaseToggled,
            scope: this,
        });
        this.db.url = fieldset.add({
            xtype: 'textfield',
            fieldLabel: _('From:'),
            labelSeparator: '',
            width: 260,
        });
        fieldset.add({
            xtype: 'label',
            text: _(
                'The default is DB-IP\u2019s free country database, which is CC BY 4.0 and needs no account. {YYYY-MM} in the address is filled in: the file is published monthly.'
            ),
            style: 'display: block; margin: 2px 0 6px 0; opacity: 0.72;',
        });
        this.db.status = fieldset.add({
            xtype: 'label',
            text: _('Not downloaded.'),
            style: 'display: block; opacity: 0.72;',
        });

        this.setDatabaseEnabled(false);
        this.addBackupBox();

        this.on('show', this.onPageShow, this);
    },

    onPageShow: function () {
        if (this.dbLoaded) return;
        this.dbLoaded = true;
        deluge.client.core.get_config_value('countrydb', {
            success: function (settings) {
                settings = settings || {};
                this.db.enabled.setValue(settings['enabled'] === true);
                this.db.url.setValue(Ext.value(settings['url'], ''));

                var last = Number(settings['last_update'] || 0);
                this.db.status.setText(
                    last > 0
                        ? String.format(
                              _('Last downloaded {0}.'),
                              new Date(last * 1000).toLocaleString()
                          )
                        : _('Not downloaded.')
                );
                this.onDatabaseToggled();
            },
            failure: function () {
                this.onDatabaseToggled();
            },
            scope: this,
        });
    },

    /**
     * Keeping a copy of the settings, and putting one back.
     *
     * The settings, not the library: the labels, the tracker rules, the stuck
     * rule, the notifications, the watched folders, the block list and the
     * schedule all live in the daemon's configuration, and the file carries
     * all of it. The torrents are listed in it rather than exported, because a
     * torrent is its file and its resume data and a list of names cannot bring
     * either back — what the list is for is knowing what you had.
     */
    addBackupBox: function () {
        var box = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('Backup'),
            autoHeight: true,
            style: 'padding-top: 5px; margin-bottom: 0px;',
        });

        box.add({
            xtype: 'label',
            text: _(
                'A copy of everything set here and in the daemon, as one file. Passwords are left out on purpose: a backup that carries them is a credential sitting in a downloads folder, so restoring one means typing those again.'
            ),
            style: 'display: block; margin-bottom: 6px; opacity: 0.72;',
        });

        box.add({
            xtype: 'button',
            text: _('Download a backup'),
            style: 'margin-bottom: 6px;',
            handler: this.onBackup,
            scope: this,
        });

        // A file input rather than a dialog of our own: the browser already
        // has one, and it is the one people recognise.
        this.restoreInput = Ext.DomHelper.append(
            box.body || Ext.getBody(),
            { tag: 'input', type: 'file', accept: '.json,application/json', style: 'display: none' },
            true
        );
        this.restoreInput.on('change', this.onRestoreChosen, this);

        box.add({
            xtype: 'button',
            text: _('Restore from a file'),
            handler: function () {
                this.restoreInput.dom.value = '';
                this.restoreInput.dom.click();
            },
            scope: this,
        });
    },

    onBackup: function () {
        deluge.client.web.export_config({
            success: function (backup) {
                var when = new Date().toISOString().slice(0, 10);
                var blob = new Blob([JSON.stringify(backup, null, 2)], {
                    type: 'application/json',
                });
                var url = URL.createObjectURL(blob);
                var link = document.createElement('a');
                link.href = url;
                link.download = 'redeluge-settings-' + when + '.json';
                document.body.appendChild(link);
                link.click();
                document.body.removeChild(link);
                // Revoked late: Safari has been known to start the download
                // after the click returns.
                setTimeout(function () { URL.revokeObjectURL(url); }, 10000);
            },
            failure: function () {
                Ext.MessageBox.show({
                    title: _('Backup'),
                    msg: _('The settings could not be read.'),
                    buttons: Ext.MessageBox.OK,
                    icon: Ext.MessageBox.ERROR,
                });
            },
            scope: this,
        });
    },

    onRestoreChosen: function () {
        var file = this.restoreInput.dom.files && this.restoreInput.dom.files[0];
        if (!file) return;

        Ext.MessageBox.confirm(
            _('Restore'),
            String.format(
                _('Put the settings in {0} back? What is set now is replaced by what the file says. The torrents are not touched.'),
                Ext.util.Format.htmlEncode(file.name)
            ),
            function (answer) {
                if (answer !== 'yes') return;
                var reader = new FileReader();
                reader.onload = function () {
                    deluge.client.web.import_config(String(reader.result), {
                        success: function () {
                            Ext.MessageBox.show({
                                title: _('Restore'),
                                msg: _('The settings were put back. Reload the page to see them.'),
                                buttons: Ext.MessageBox.OK,
                                icon: Ext.MessageBox.INFO,
                            });
                        },
                        failure: function (error) {
                            Ext.MessageBox.show({
                                title: _('Restore'),
                                msg: (error && error.error && error.error.message) || _('That file could not be read as a backup.'),
                                buttons: Ext.MessageBox.OK,
                                icon: Ext.MessageBox.ERROR,
                            });
                        },
                    });
                };
                reader.readAsText(file);
            },
            this
        );
    },

    onDatabaseToggled: function () {
        this.setDatabaseEnabled(this.db.enabled.getValue() === true);
    },

    setDatabaseEnabled: function (on) {
        this.db.url.setDisabled(!on);
    },

    onApply: function () {
        // Nothing read means nothing of this page's to write.
        if (!this.dbLoaded) return;
        deluge.client.core.set_config({
            countrydb: {
                enabled: this.db.enabled.getValue() === true,
                url: this.db.url.getValue() || '',
            },
        });
    },
});
